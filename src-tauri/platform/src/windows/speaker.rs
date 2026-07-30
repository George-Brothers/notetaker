//! System audio on Windows, via WASAPI loopback.
//!
//! "Loopback" is the slightly odd trick at the heart of this: you open the
//! default *render* (playback) device, then initialize its client in the
//! *capture* direction. Windows then hands you everything that device is
//! playing. It needs no permission prompt and no virtual audio driver, which
//! makes Windows the easier of the two platforms — macOS requires
//! ScreenCaptureKit and an explicit Screen Recording grant for the same thing.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/audio-actual/src/speaker/windows.rs`. See the NOTICE file.
//! Their version exposes an async `Stream` built on their own workspace crates
//! and takes the first channel only; this drains synchronously through
//! [`crate::ring`] and downmixes properly through [`crate::convert`].
//!
//! Compile-verified against `x86_64-pc-windows-msvc` from Linux. Whether the
//! device actually delivers audio is first learned on a real machine.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use wasapi::{initialize_mta, Direction, SampleType, ShareMode, StreamMode, WaveFormat};

use crate::convert::{to_mono_f32, SampleFormat};
use crate::resample::Resampler;
use crate::ring::{self, RingReader, RingWriter};
use crate::TARGET_SAMPLE_RATE;

/// How long to wait for the capture thread to negotiate a format and start.
/// Past this something is wrong with the audio stack, and hanging the record
/// button forever is the one outcome the user cannot diagnose.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to block waiting for the device to signal new audio, per iteration.
/// Also the granularity at which the thread notices it has been asked to stop.
const EVENT_TIMEOUT_MS: u32 = 250;

/// The interleaved layout the device negotiated.
#[derive(Debug, Clone, Copy)]
struct DeviceFormat {
    sample_rate: u32,
    channels: usize,
    format: SampleFormat,
}

/// System audio as a pull-based source.
pub struct SystemAudioSource {
    reader: RingReader,
    resampler: Resampler,
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    label: String,
}

impl SystemAudioSource {
    /// Opens the default playback device in loopback mode and starts capturing.
    ///
    /// Blocks until the format is negotiated, so that a failure to open the
    /// device is reported here — where the caller can put it in front of the
    /// user before the recording starts — rather than surfacing later as a
    /// silent track.
    pub fn start() -> Result<Self> {
        let running = Arc::new(AtomicBool::new(true));
        let (writer, reader) = ring::channel(ring::DEFAULT_CAPACITY);
        let (init_tx, init_rx) = mpsc::channel();

        let thread = {
            let running = running.clone();
            thread::Builder::new()
                .name("notetaker-wasapi-loopback".into())
                .spawn(move || {
                    let writer_finish = capture_loop(writer, running, init_tx);
                    if let Err(e) = writer_finish {
                        log::warn!("system audio capture ended: {e:#}");
                    }
                })
                .context("starting the system-audio capture thread")?
        };

        // The thread reports its negotiated format, or why it could not start.
        let format = match init_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(format)) => format,
            Ok(Err(e)) => {
                running.store(false, Ordering::Release);
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                running.store(false, Ordering::Release);
                let _ = thread.join();
                anyhow::bail!(
                    "Windows did not start sharing this computer's sound within {} seconds.",
                    START_TIMEOUT.as_secs()
                );
            }
        };

        log::info!(
            "system audio: {} Hz, {} channel(s), {:?}",
            format.sample_rate,
            format.channels,
            format.format
        );

        Ok(Self {
            reader,
            resampler: Resampler::new(format.sample_rate, TARGET_SAMPLE_RATE)?,
            running,
            thread: Some(thread),
            label: "this computer's sound".to_string(),
        })
    }

    /// Appends whatever audio is ready, resampled to [`TARGET_SAMPLE_RATE`].
    pub fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let mut raw = Vec::new();
        self.reader.drain(&mut raw);
        self.resampler.push(&raw);
        self.resampler.drain(out)?;
        Ok(())
    }

    /// True once the device is gone and everything buffered has been read.
    pub fn is_finished(&self) -> bool {
        self.reader.is_finished()
    }

    /// Stops the capture thread and releases the device.
    pub fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        if let Some(t) = self.thread.take() {
            // The thread wakes at least every EVENT_TIMEOUT_MS, so this is a
            // bounded wait rather than an open-ended one.
            let _ = t.join();
        }
        Ok(())
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Samples discarded because the session loop could not keep up. Non-zero
    /// means the recording is missing audio and should say so.
    pub fn dropped_samples(&self) -> usize {
        self.reader.dropped()
    }
}

impl Drop for SystemAudioSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// The capture thread: negotiates a format, reports it, then pumps audio into
/// the ring until asked to stop.
fn capture_loop(
    mut writer: RingWriter,
    running: Arc<AtomicBool>,
    init_tx: mpsc::Sender<Result<DeviceFormat>>,
) -> Result<()> {
    let started = open_loopback();
    let (audio_client, event, capture_client, format) = match started {
        Ok(v) => v,
        Err(e) => {
            // Report the failure, then mark the stream over so a caller that
            // raced past the timeout still sees a finished source rather than
            // waiting on audio that will never arrive.
            let _ = init_tx.send(Err(e));
            writer.finish();
            return Ok(());
        }
    };
    let _ = init_tx.send(Ok(format));

    let mut bytes = VecDeque::new();
    let mut mono = Vec::new();

    while running.load(Ordering::Acquire) {
        // A timeout is normal: it just means the device played nothing.
        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            continue;
        }
        bytes.clear();
        if let Err(e) = capture_client.read_from_device_to_deque(&mut bytes) {
            log::warn!("reading system audio: {e}");
            continue;
        }
        if bytes.is_empty() {
            continue;
        }
        mono.clear();
        to_mono_f32(
            bytes.make_contiguous(),
            format.format,
            format.channels,
            &mut mono,
        );
        writer.write(&mono);
    }

    writer.finish();
    let _ = audio_client.stop_stream();
    Ok(())
}

/// Opens the default render device for loopback capture.
type Loopback = (
    wasapi::AudioClient,
    wasapi::Handle,
    wasapi::AudioCaptureClient,
    DeviceFormat,
);

fn open_loopback() -> Result<Loopback> {
    // Multithreaded COM apartment: this thread is not a UI thread and must not
    // pump messages.
    initialize_mta()
        .ok()
        .context("Windows would not start its audio system (COM initialization failed).")?;

    // Render, not Capture: the loopback trick is a *playback* device opened in
    // the capture direction below.
    let device = wasapi::get_default_device(&Direction::Render)
        .context("This computer has no active speakers or headphones to record from.")?;
    let mut audio_client = device
        .get_iaudioclient()
        .context("Windows would not let Notetaker open the speakers for recording.")?;

    // Ask for 32-bit float at the device's own rate and channel layout. Asking
    // for the mix format's rate rather than 16 kHz keeps Windows out of the
    // resampling business — `crate::resample` does that with a filter we can
    // test, instead of whatever the driver happens to do.
    let mix = audio_client
        .get_mixformat()
        .context("Windows would not report the speakers' audio format.")?;
    let desired = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        mix.get_samplespersec() as usize,
        mix.get_nchannels() as usize,
        Some(mix.get_dwchannelmask()),
    );
    // If the device refuses float, fall back to whatever it proposes instead —
    // `convert` handles the integer layouts too.
    let accepted = audio_client
        .is_supported(&desired, &ShareMode::Shared)
        .context("Windows would not agree on an audio format for the speakers.")?
        .unwrap_or(desired);

    let format = DeviceFormat {
        sample_rate: accepted.get_samplespersec(),
        channels: accepted.get_nchannels() as usize,
        format: sample_format(&accepted)?,
    };

    let (_default_period, min_period) = audio_client
        .get_device_period()
        .context("Windows would not report the speakers' buffer timing.")?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_period,
    };
    audio_client
        .initialize_client(&accepted, &Direction::Capture, &mode)
        .context("Windows would not start recording this computer's sound.")?;

    let event = audio_client
        .set_get_eventhandle()
        .context("Windows would not signal when new sound is ready.")?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .context("Windows would not hand over the sound it is playing.")?;
    audio_client
        .start_stream()
        .context("Windows would not start the speaker recording stream.")?;

    Ok((audio_client, event, capture_client, format))
}

/// Maps the negotiated WASAPI format onto one [`crate::convert`] understands.
///
/// An unrecognized layout is an error rather than a guess: guessing produces
/// static, and static in a recording of a meeting is indistinguishable from a
/// broken microphone.
fn sample_format(format: &WaveFormat) -> Result<SampleFormat> {
    let bits = format.get_bitspersample();
    let ty = format
        .get_subformat()
        .context("Windows reported an audio format Notetaker does not recognize.")?;
    match (ty, bits) {
        (SampleType::Float, 32) => Ok(SampleFormat::F32),
        (SampleType::Int, 16) => Ok(SampleFormat::I16),
        (SampleType::Int, 32) => Ok(SampleFormat::I32),
        (ty, bits) => anyhow::bail!(
            "This computer's speakers use an audio format Notetaker cannot read yet \
             ({ty:?}, {bits}-bit)."
        ),
    }
}
