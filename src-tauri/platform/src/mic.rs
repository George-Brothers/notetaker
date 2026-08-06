//! The microphone, on both shipping platforms.
//!
//! `cpal` already abstracts WASAPI and CoreAudio, so unlike system audio this
//! needs no per-OS split — the same code covers Mac and PC. What it does need
//! is the same treatment as everything else here: the device picks the sample
//! rate and channel count, and both get normalized to mono at
//! [`TARGET_SAMPLE_RATE`] through tested code rather than inside the callback.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/audio-actual/src/mic.rs`. See the NOTICE file.
//!
//! Compile-verified for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`
//! from Linux. `cpal` is not built on Linux at all (its `alsa-sys` needs
//! pkg-config we have no sudo for), so this module is absent there and the
//! capture logic above it is tested against core's `FakeSource` instead.
//!
//! # Why the stream lives on its own thread
//!
//! `cpal::Stream` is **not `Send` on macOS** — CoreAudio ties it to the thread
//! that created it — while `AudioSource` requires `Send`. Holding the stream in
//! this struct therefore compiled on Windows and failed on macOS, which is
//! precisely the class of bug the cross-target `cargo check` cannot catch: the
//! platform crate alone is fine, and the error only appears where *core* uses
//! `MicSource` as a `dyn AudioSource`. Core cannot be cross-checked, so CI found
//! it on the first macOS build this project ever ran.
//!
//! The fix is not a `cfg` or an `unsafe impl Send`: the stream is never stored
//! at all. A worker thread opens the device, plays it, and parks until asked to
//! stop, so everything left in `MicSource` is `Send` by construction on every
//! platform rather than by luck on one of them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::convert::interleaved_f32_to_mono;
use crate::resample::Resampler;
use crate::ring::{self, RingReader};
use crate::TARGET_SAMPLE_RATE;

/// The microphone as a pull-based source.
pub struct MicSource {
    reader: RingReader,
    resampler: Resampler,
    /// Tells the worker thread to release the device. See the module note.
    stop: Arc<AtomicBool>,
    /// Joined on `stop`, so the device is actually released before we return.
    worker: Option<std::thread::JoinHandle<()>>,
    /// Set by the error callback when the OS reports the device is gone.
    device_lost: Arc<AtomicBool>,
    label: String,
}

/// What the worker reports back once the device is open, so `start` can fail
/// with the device's own error rather than succeeding into a silent recording.
struct Opened {
    name: String,
    sample_rate: u32,
}

/// How often the worker checks whether it has been asked to stop. Short enough
/// that releasing the microphone feels immediate, long enough to cost nothing.
const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(50);

impl MicSource {
    /// Opens the system default input device and starts capturing.
    ///
    /// Blocks until the worker has either opened the device or failed, so a
    /// missing or busy microphone is an error from this call rather than a
    /// recording that turns out to be silent.
    pub fn start() -> Result<Self> {
        Self::start_preferred(&[])
    }

    /// Opens the first named input device that is available, falling back to
    /// the system default when none of the saved choices is connected.
    pub fn start_preferred(preferred_devices: &[String]) -> Result<Self> {
        let preferred_devices = preferred_devices.to_vec();
        let (mut writer, reader) = ring::channel(ring::DEFAULT_CAPACITY);
        let device_lost = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Opened>>();
        let err_lost = device_lost.clone();
        let worker_stop = stop.clone();

        // Everything cpal touches happens in here, and the stream never leaves.
        let worker = std::thread::Builder::new()
            .name("notetaker-mic".into())
            .spawn(move || {
                let opened = (|| -> Result<(cpal::Stream, Opened)> {
                    let host = cpal::default_host();
                    let device = preferred_devices
                        .iter()
                        .find_map(|preferred| {
                            host.input_devices().ok()?.find(|device| {
                                device.name().ok().as_deref() == Some(preferred.as_str())
                            })
                        })
                        .or_else(|| host.default_input_device())
                        .context("No microphone is available on this computer.")?;
                    let name = device
                        .name()
                        .unwrap_or_else(|_| "the microphone".to_string());
                    let config = device.default_input_config().with_context(|| {
                        format!("Notetaker could not read the settings for {name}.")
                    })?;

                    let sample_rate = config.sample_rate().0;
                    let channels = config.channels() as usize;
                    let sample_format = config.sample_format();

                    let on_error = move |e: cpal::StreamError| {
                        log::warn!("microphone stream error: {e}");
                        err_lost.store(true, Ordering::Release);
                    };

                    // One closure per sample format cpal may hand us. Each does
                    // the same two steps: widen to `f32`, then downmix to mono.
                    // The arithmetic lives in `convert`, which is tested;
                    // nothing here touches samples except to widen them.
                    // Buffers are declared per arm and reused across callbacks,
                    // so a steady stream does not allocate on the audio thread.
                    let stream = match sample_format {
                        cpal::SampleFormat::F32 => {
                            let mut mono = Vec::new();
                            device.build_input_stream(
                                &config.into(),
                                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                    mono.clear();
                                    interleaved_f32_to_mono(data, channels, &mut mono);
                                    writer.write(&mono);
                                },
                                on_error,
                                None,
                            )
                        }
                        cpal::SampleFormat::I16 => {
                            let (mut wide, mut mono) = (Vec::new(), Vec::new());
                            device.build_input_stream(
                                &config.into(),
                                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                                    wide.clear();
                                    // 32768, not 32767, so i16::MIN maps to
                                    // exactly -1.0.
                                    wide.extend(data.iter().map(|s| *s as f32 / 32_768.0));
                                    mono.clear();
                                    interleaved_f32_to_mono(&wide, channels, &mut mono);
                                    writer.write(&mono);
                                },
                                on_error,
                                None,
                            )
                        }
                        cpal::SampleFormat::U16 => {
                            let (mut wide, mut mono) = (Vec::new(), Vec::new());
                            device.build_input_stream(
                                &config.into(),
                                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                                    wide.clear();
                                    // u16 is offset binary: 32768 is silence,
                                    // not zero. Getting this wrong yields a
                                    // colossal DC offset, which sounds like
                                    // silence but wrecks every level check
                                    // downstream.
                                    wide.extend(
                                        data.iter().map(|s| (*s as f32 - 32_768.0) / 32_768.0),
                                    );
                                    mono.clear();
                                    interleaved_f32_to_mono(&wide, channels, &mut mono);
                                    writer.write(&mono);
                                },
                                on_error,
                                None,
                            )
                        }
                        other => anyhow::bail!(
                            "{name} uses an audio format Notetaker cannot read yet ({other:?})."
                        ),
                    }
                    .with_context(|| format!("Notetaker could not start recording from {name}."))?;

                    stream
                        .play()
                        .with_context(|| format!("Notetaker could not start {name}."))?;

                    log::info!(
                        "microphone: {name}, {sample_rate} Hz, {channels} channel(s), \
                         {sample_format:?}"
                    );
                    Ok((stream, Opened { name, sample_rate }))
                })();

                match opened {
                    Ok((stream, info)) => {
                        // If the receiver is gone the caller gave up; drop the
                        // stream rather than holding the device open forever.
                        if ready_tx.send(Ok(info)).is_err() {
                            return;
                        }
                        while !worker_stop.load(Ordering::Acquire) {
                            std::thread::sleep(STOP_POLL);
                        }
                        // Pausing first makes sure no callback is mid-write
                        // when the stream goes.
                        let _ = stream.pause();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .context("Notetaker could not start the microphone thread.")?;

        // A worker that died before reporting leaves a disconnected channel;
        // say something a user can act on rather than surfacing a recv error.
        let opened = ready_rx.recv().map_err(|_| {
            anyhow::anyhow!("The microphone stopped before it started recording.")
        })??;

        Ok(Self {
            reader,
            resampler: Resampler::new(opened.sample_rate, TARGET_SAMPLE_RATE)?,
            stop,
            worker: Some(worker),
            device_lost,
            label: opened.name,
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
    ///
    /// Checks the buffer before reporting finished: a microphone unplugged
    /// mid-sentence should still contribute the words it did capture.
    pub fn is_finished(&self) -> bool {
        if self.device_lost.load(Ordering::Acquire) && self.reader.available() == 0 {
            return true;
        }
        self.reader.is_finished()
    }

    /// Closes the device.
    ///
    /// Joins the worker rather than only signalling it, so that when this
    /// returns the microphone really is released — otherwise starting a second
    /// recording could race the first one's teardown for the device.
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // A panicked worker has already dropped its stream by unwinding;
            // there is nothing left to report and nothing to clean up.
            let _ = worker.join();
        }
        Ok(())
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Samples discarded because the session loop could not keep up.
    pub fn dropped_samples(&self) -> usize {
        self.reader.dropped()
    }

    /// The worker can end because the input device disappeared, which is
    /// different from a user releasing the dictation key. Core uses this to
    /// reject a partial utterance rather than silently pasting one.
    pub fn failure_message(&self) -> Option<String> {
        self.device_lost
            .load(Ordering::Acquire)
            .then(|| format!("{} disconnected while dictating", self.label))
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
