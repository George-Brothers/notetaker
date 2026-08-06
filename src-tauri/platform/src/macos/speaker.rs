//! System audio on macOS, via ScreenCaptureKit.
//!
//! This is the Mac half of "record the other side of a call". Windows gets it
//! from WASAPI loopback with no permission prompt at all; macOS has no loopback
//! device, and the only supported way to hear what the speakers are playing is
//! to ask ScreenCaptureKit for a capture session and take its audio while
//! throwing the video away. That is why this needs the **Screen Recording**
//! permission for something the user experiences as recording sound.
//!
//! Until 2026-08-05 this file held a design and a deliberate error return,
//! because it could not be written honestly from the Linux box the rest of the
//! project was built on: everything else here is compile-verified against
//! `aarch64-apple-darwin` from anywhere, but an Objective-C delegate class and
//! a TCC permission are exactly the things that compile cleanly and then crash,
//! or silently receive nothing. It is written now because there is a Mac to run
//! it on.
//!
//! # The shape, and why it matches Windows
//!
//! Everything below the callback — the ring buffer, the downmix, the resample —
//! is shared, pure and tested. So this file is only the Apple-specific glue:
//!
//! - `SCShareableContent` to find a display. Any display will do; we want its
//!   audio, not its pixels. This is also the call that fails when Screen
//!   Recording has not been granted, which makes it the permission check.
//! - `SCContentFilter` over that display, and an `SCStreamConfiguration` with
//!   `capturesAudio = true` and `excludesCurrentProcessAudio = true` — without
//!   the latter Notetaker records its own notification sounds back into the
//!   meeting.
//! - The video side is **minimised, not disabled**: ScreenCaptureKit will not
//!   run a stream with no video, so it is configured at 2x2 pixels one frame
//!   every two seconds, and every video sample is dropped on arrival.
//! - A delegate class defined at runtime with [`objc2::define_class`],
//!   conforming to `SCStreamOutput`, which pushes audio into [`crate::ring`].
//!
//! # Two things here are load-bearing and were nearly wrong
//!
//! **The stream never leaves its worker thread.** `AudioSource` in core requires
//! `Send`, and none of these Objective-C objects are. This is the same trap that
//! caught `MicSource`, where `cpal::Stream` is `!Send` on macOS only — a bug the
//! cross-target `cargo check` structurally cannot see, because it appears where
//! *core* uses the type as a `dyn AudioSource` and core cannot be
//! cross-checked. So, exactly as in [`crate::mic`], the `SCStream` is created,
//! held and stopped entirely inside one thread and never stored in this struct.
//! What is left is `Send` by construction rather than by luck.
//!
//! **ScreenCaptureKit delivers planar audio, not interleaved.** The
//! `AudioBufferList` holds one buffer *per channel* (`LLL`, `RRR`), not
//! interleaved frames (`LRLR`). Reading it the interleaved way does not fail —
//! it produces a recording at half the length that plays at double speed, which
//! is indistinguishable from a broken device. Hence
//! [`crate::convert::planar_f32_to_mono`], which is a tested pure function and
//! not arithmetic inlined into a callback.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamErrorCode,
    SCStreamOutput, SCStreamOutputType,
};

use crate::convert::planar_f32_to_mono;
use crate::resample::Resampler;
use crate::ring::{self, RingReader, RingWriter};
use crate::TARGET_SAMPLE_RATE;

/// The rate we ask ScreenCaptureKit for. It is resampled to
/// [`TARGET_SAMPLE_RATE`] by [`crate::resample`], whose 48 k -> 16 k path is
/// already covered by tests — the same division of labour as on Windows, where
/// we also decline to let the OS do the resampling.
const CAPTURE_SAMPLE_RATE: u32 = 48_000;

/// Stereo in, mono out. Asking for two and averaging loses nothing; asking for
/// one lets CoreAudio decide how to fold them, which is not a decision we can
/// test.
const CAPTURE_CHANNELS: usize = 2;

/// The most channels a single `AudioBufferList` may deliver before we stop
/// reading. Two is what we ask for; the headroom costs a few hundred bytes of
/// stack and means an aggregate device that ignores `channelCount` truncates
/// instead of reading out of bounds.
const MAX_CHANNELS: usize = 8;

/// Asking for more channels than the buffer list can hold would silently
/// truncate the audio. A `const` assertion rather than a test, because the
/// relationship is between two constants and so belongs to compilation.
const _: () = assert!(
    CAPTURE_CHANNELS >= 1 && CAPTURE_CHANNELS <= MAX_CHANNELS,
    "the channel count we request does not fit the AudioBufferList we provide"
);

/// How long to wait for ScreenCaptureKit to answer. Past this something is
/// wrong, and a Record button that hangs forever is the one failure a user
/// cannot diagnose. Generous because the very first call in a process is what
/// raises the permission dialog, and the user has to read it.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the worker checks whether it has been asked to stop.
const STOP_POLL: Duration = Duration::from_millis(100);

/// System audio as a pull-based source.
///
/// Deliberately holds no Objective-C object — see the module docs. Everything
/// in here is `Send` on its own merits.
pub struct SystemAudioSource {
    reader: RingReader,
    resampler: Resampler,
    /// Tells the worker to stop the stream and release the display.
    stop: Arc<AtomicBool>,
    /// Joined on `stop`, so capture has really ended before we return.
    worker: Option<thread::JoinHandle<()>>,
    /// Samples the delegate could not fit into the ring. Lives here rather than
    /// on the reader because the delegate counts them before they are written.
    overflowed: Arc<AtomicUsize>,
    label: String,
}

impl SystemAudioSource {
    /// Starts capturing this computer's sound.
    ///
    /// Blocks until ScreenCaptureKit has either started the stream or refused,
    /// so a missing permission is reported *here* — in front of the user,
    /// before the recording begins — rather than becoming a silent track that
    /// is only discovered after the meeting.
    pub fn start() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let overflowed = Arc::new(AtomicUsize::new(0));
        let (writer, reader) = ring::channel(ring::DEFAULT_CAPACITY);
        let (init_tx, init_rx) = mpsc::channel();

        let worker = {
            let stop = stop.clone();
            let overflowed = overflowed.clone();
            thread::Builder::new()
                .name("notetaker-sck-audio".into())
                .spawn(move || capture_thread(writer, stop, overflowed, init_tx))
                .context("Notetaker could not start the system-audio thread.")?
        };

        match init_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(e);
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                anyhow::bail!(
                    "This computer did not start sharing its sound within {} seconds. \
                     If a permission window appeared, try recording again after allowing it.",
                    START_TIMEOUT.as_secs()
                );
            }
        }

        Ok(Self {
            reader,
            resampler: Resampler::new(CAPTURE_SAMPLE_RATE, TARGET_SAMPLE_RATE)?,
            stop,
            worker: Some(worker),
            overflowed,
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

    /// True once the stream has ended and everything buffered has been read.
    pub fn is_finished(&self) -> bool {
        self.reader.is_finished()
    }

    /// Stops the stream and releases the display.
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // The worker wakes at least every STOP_POLL, so this is bounded.
            let _ = worker.join();
        }
        Ok(())
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Samples lost because the session loop could not keep up. Non-zero means
    /// the recording is missing audio and should say so.
    pub fn dropped_samples(&self) -> usize {
        self.reader.dropped() + self.overflowed.load(Ordering::Relaxed)
    }
}

impl Drop for SystemAudioSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// --- the delegate ---------------------------------------------------------

/// What the delegate owns. The `Mutex` is not contention management — the
/// callbacks arrive on one serial queue — it is what makes the writer legal to
/// touch from a thread other than the one that built it.
struct OutputIvars {
    writer: Mutex<RingWriter>,
    overflowed: Arc<AtomicUsize>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NotetakerSystemAudioOutput"]
    #[ivars = OutputIvars]
    struct AudioOutput;

    unsafe impl NSObjectProtocol for AudioOutput {}

    unsafe impl SCStreamOutput for AudioOutput {
        /// One buffer of audio (or a video frame we throw away).
        ///
        /// Called on our own serial queue. It must not block or allocate more
        /// than necessary: this is the audio path, and time spent here is time
        /// ScreenCaptureKit is not filling the next buffer.
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output_sample_buffer(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            // The video stream exists only because ScreenCaptureKit will not
            // run without one. Every frame is discarded, unlooked at.
            if output_type != SCStreamOutputType::Audio {
                return;
            }

            let mut mono = Vec::new();
            // SAFETY: `sample_buffer` is the buffer ScreenCaptureKit just handed
            // us and is valid for this call.
            let ok = unsafe { copy_audio(sample_buffer, &mut mono) };
            if !ok || mono.is_empty() {
                return;
            }

            let ivars = self.ivars();
            // A poisoned lock means a previous callback panicked. Dropping the
            // audio is right: the alternative is panicking across the
            // Objective-C frame, which is undefined behaviour.
            let Ok(mut writer) = ivars.writer.lock() else {
                return;
            };
            let written = writer.write(&mono);
            if written < mono.len() {
                ivars
                    .overflowed
                    .fetch_add(mono.len() - written, Ordering::Relaxed);
            }
        }
    }
);

impl AudioOutput {
    fn new(writer: RingWriter, overflowed: Arc<AtomicUsize>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars {
            writer: Mutex::new(writer),
            overflowed,
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Pulls planar `f32` audio out of a `CMSampleBuffer` and downmixes it to mono.
///
/// Returns false if the buffer could not be read, which the caller treats as
/// "skip this one" rather than as a reason to tear the recording down — a
/// single unreadable buffer is a glitch, and stopping a meeting recording over
/// it would be a worse outcome than a gap of a few milliseconds.
///
/// # Safety
///
/// `sample_buffer` must be a valid `CMSampleBuffer` carrying audio.
unsafe fn copy_audio(sample_buffer: &CMSampleBuffer, out: &mut Vec<f32>) -> bool {
    // `AudioBufferList` is a C variable-length struct: a count followed by that
    // many `AudioBuffer`s. Rust's binding declares exactly one, so a fixed
    // backing struct of MAX_CHANNELS is allocated and its address handed over
    // as an `AudioBufferList` of that size. This mirrors the C idiom exactly;
    // the layouts are identical because both are `#[repr(C)]` with the same
    // members in the same order.
    #[repr(C)]
    struct AudioBufferListN {
        number_buffers: u32,
        buffers: [AudioBuffer; MAX_CHANNELS],
    }

    let mut list = AudioBufferListN {
        number_buffers: 0,
        buffers: [AudioBuffer {
            mNumberChannels: 0,
            mDataByteSize: 0,
            mData: std::ptr::null_mut(),
        }; MAX_CHANNELS],
    };
    let mut block_buffer = std::ptr::null_mut();

    // Two passes, which is the documented idiom and — as this cost an
    // afternoon to learn — is not optional.
    //
    // The obvious thing is to hand over the whole `MAX_CHANNELS` struct and let
    // CoreMedia fill in as much as it needs. That fails with
    // `kCMSampleBufferError_ArrayTooSmall` (-12737) on every single buffer,
    // which is a thoroughly misleading name: the size passed must describe a
    // list of *exactly* the number of buffers the sample holds, not merely one
    // big enough to contain it. Passing 136 bytes when it wants 40 is "too
    // small". The symptom is a stream that starts, calls back at the right
    // rate, and yields zero samples forever.
    //
    // So: ask first, then fill.
    let mut needed: usize = 0;
    let probe = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            std::ptr::null_mut(),
            0,
            None,
            None,
            0,
            std::ptr::null_mut(),
        )
    };
    if probe != 0 || needed == 0 || needed > std::mem::size_of::<AudioBufferListN>() {
        return false;
    }

    // The returned `block_buffer` owns the samples. It must outlive our reads,
    // and must be released afterwards — hence the explicit `Retained` below
    // rather than letting it leak on every callback.
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            std::ptr::null_mut(),
            &mut list as *mut AudioBufferListN as *mut AudioBufferList,
            needed,
            None,
            None,
            0,
            &mut block_buffer,
        )
    };

    if status != 0 || block_buffer.is_null() {
        return false;
    }
    // Takes ownership of the +1 retain the call above returned, so it is
    // released when this function ends.
    let _block_buffer = unsafe { Retained::from_raw(block_buffer) };

    let count = (list.number_buffers as usize).min(MAX_CHANNELS);
    if count == 0 {
        return false;
    }

    let mut planes: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
    for (plane, buffer) in planes.iter_mut().zip(list.buffers.iter()).take(count) {
        if buffer.mData.is_null() {
            return false;
        }
        let samples = buffer.mDataByteSize as usize / std::mem::size_of::<f32>();
        // SAFETY: CoreAudio guarantees `mData` points to `mDataByteSize` bytes,
        // and ScreenCaptureKit's audio format is 32-bit float. The buffers are
        // 16-byte aligned, so the `f32` alignment requirement is met.
        *plane = unsafe { std::slice::from_raw_parts(buffer.mData as *const f32, samples) };
    }

    // Planar, not interleaved. See the module docs — this is the distinction
    // that silently doubles playback speed when it is got wrong.
    planar_f32_to_mono(&planes[..count], out);
    true
}

// --- the worker thread ----------------------------------------------------

/// Builds the stream, starts it, and holds it until asked to stop.
///
/// Everything Objective-C lives and dies inside this function, which is what
/// keeps [`SystemAudioSource`] `Send`.
fn capture_thread(
    writer: RingWriter,
    stop: Arc<AtomicBool>,
    overflowed: Arc<AtomicUsize>,
    init_tx: mpsc::Sender<Result<()>>,
) {
    let started = start_stream(writer, overflowed);
    let (stream, _output, _queue) = match started {
        Ok(parts) => {
            let _ = init_tx.send(Ok(()));
            parts
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };

    while !stop.load(Ordering::Acquire) {
        thread::sleep(STOP_POLL);
    }

    // Stopping is asynchronous too, but we do not wait on it: the ring is
    // already finished below, the caller is joining this thread, and a stop
    // that takes an extra moment to land costs nothing. Waiting could hang the
    // stop button, which is the failure that actually matters.
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |_error: *mut objc2_foundation::NSError| {
        let _ = tx.send(());
    });
    unsafe { stream.stopCaptureWithCompletionHandler(Some(&handler)) };
    let _ = rx.recv_timeout(Duration::from_secs(2));
}

/// Everything the worker must keep alive for capture to continue.
///
/// The delegate and the queue are not referenced again, but dropping either
/// would end the stream — ScreenCaptureKit does not retain the output object,
/// so it is held here for as long as the recording lasts.
type StreamParts = (
    Retained<SCStream>,
    Retained<AudioOutput>,
    dispatch2::DispatchRetained<DispatchQueue>,
);

fn start_stream(writer: RingWriter, overflowed: Arc<AtomicUsize>) -> Result<StreamParts> {
    let display = first_display()?;

    // No windows excluded: we want the whole display's audio, and its pixels
    // are discarded regardless.
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &NSArray::new(),
        )
    };

    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setCapturesAudio(true);
        config.setSampleRate(CAPTURE_SAMPLE_RATE as isize);
        config.setChannelCount(CAPTURE_CHANNELS as isize);
        // Without this, Notetaker records its own notification sounds — and,
        // when the recording is played back inside the app, itself.
        config.setExcludesCurrentProcessAudio(true);

        // The video stream cannot be switched off, so it is made as close to
        // free as the API allows: four pixels, one frame every two seconds.
        config.setWidth(2);
        config.setHeight(2);
        config.setMinimumFrameInterval(CMTime {
            value: 2,
            timescale: 1,
            flags: objc2_core_media::CMTimeFlags(1), // kCMTimeFlags_Valid
            epoch: 0,
        });
    }

    let output = AudioOutput::new(writer, overflowed);
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None)
    };

    // A serial queue: the delegate takes a lock per callback, and a concurrent
    // queue would buy nothing but contention on it.
    let queue = DispatchQueue::new("com.georgebrothers.notetaker.sck", DispatchQueueAttr::SERIAL);

    unsafe {
        stream.addStreamOutput_type_sampleHandlerQueue_error(
            ProtocolObject::from_ref(&*output),
            SCStreamOutputType::Audio,
            Some(&queue),
        )
    }
    .map_err(|e| anyhow::anyhow!(describe(&e)))
    .context("Notetaker could not attach to this computer's sound.")?;

    // Starting is asynchronous, and its error is the one that reports a refused
    // permission. Waiting for it here is what lets `start` fail in front of the
    // user rather than returning a stream that never delivers a sample.
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
        let message = if error.is_null() {
            None
        } else {
            // SAFETY: non-null means ScreenCaptureKit handed us a live error.
            Some(describe(unsafe { &*error }))
        };
        let _ = tx.send(message);
    });
    unsafe { stream.startCaptureWithCompletionHandler(Some(&handler)) };

    match rx.recv_timeout(START_TIMEOUT) {
        Ok(None) => Ok((stream, output, queue)),
        Ok(Some(message)) => Err(anyhow::anyhow!(message)),
        Err(_) => anyhow::bail!(
            "This computer did not start sharing its sound within {} seconds.",
            START_TIMEOUT.as_secs()
        ),
    }
}

/// Finds a display to attach to, and doubles as the permission check.
///
/// `SCShareableContent` is what fails when Screen Recording has not been
/// granted, so this is where a refusal is turned into something a person can
/// act on.
fn first_display() -> Result<Retained<objc2_screen_capture_kit::SCDisplay>> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut objc2_foundation::NSError| {
            // SAFETY: ScreenCaptureKit passes exactly one of these as non-null.
            let result = if !error.is_null() {
                Err(describe(unsafe { &*error }))
            } else if content.is_null() {
                Err(PERMISSION_MESSAGE.to_string())
            } else {
                let content = unsafe { &*content };
                let displays = unsafe { content.displays() };
                match displays.iter().next() {
                    Some(display) => Ok(display),
                    None => Err("Notetaker could not find a screen to listen to, so it \
                                 cannot record this computer's sound."
                        .to_string()),
                }
            };
            let _ = tx.send(result);
        },
    );

    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };

    match rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(display)) => Ok(display),
        Ok(Err(message)) => Err(anyhow::anyhow!(message)),
        Err(_) => anyhow::bail!(
            "This computer did not answer when Notetaker asked to record its sound. \
             If a permission window appeared, try recording again after allowing it."
        ),
    }
}

/// What the user is told when Screen Recording has not been granted.
///
/// It names the exact place to go, because "grant the permission" is useless to
/// someone who has never opened that pane, and it says what Notetaker does with
/// it — a request to record your screen, for an app that records sound, is
/// alarming until it is explained. No API names, no error codes.
const PERMISSION_MESSAGE: &str = "Notetaker needs permission to record this computer's sound \
     before it can record a meeting. Open System Settings > Privacy & Security > Screen & System \
     Audio Recording, turn Notetaker on, then quit and reopen Notetaker and start the recording \
     again. Notetaker only listens to the sound; it never records the picture on your screen.";

/// Turns an `NSError` into something written for a person.
///
/// Only `UserDeclined` gets bespoke wording, because it is the only one the
/// user can actually fix and it is by far the most likely. Everything else
/// keeps the system's own description, which is more use than a message that
/// pretends to know what went wrong.
fn describe(error: &objc2_foundation::NSError) -> String {
    if error.code() == SCStreamErrorCode::UserDeclined.0 {
        return PERMISSION_MESSAGE.to_string();
    }
    format!(
        "Notetaker could not record this computer's sound. {}",
        error.localizedDescription()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate we ask for must be one [`crate::resample`] can actually take to
    /// [`TARGET_SAMPLE_RATE`]. A mismatch here would be found at the first
    /// recording rather than at build time.
    #[test]
    fn the_capture_rate_resamples_to_the_target_rate() {
        let resampler = Resampler::new(CAPTURE_SAMPLE_RATE, TARGET_SAMPLE_RATE)
            .expect("48 kHz -> 16 kHz is a supported conversion");
        assert_eq!(resampler.from_hz(), CAPTURE_SAMPLE_RATE);
        assert_eq!(resampler.to_hz(), TARGET_SAMPLE_RATE);
        assert!(!resampler.is_passthrough());
    }

    /// The permission message is the entire user experience of a refused grant,
    /// so it is pinned: it must name where to go and must not leak API names.
    #[test]
    fn the_permission_message_says_where_to_go_and_names_no_apis() {
        assert!(PERMISSION_MESSAGE.contains("System Settings"));
        assert!(PERMISSION_MESSAGE.contains("Screen & System Audio Recording"));
        for jargon in [
            "ScreenCaptureKit",
            "SCStream",
            "TCC",
            "delegate",
            "API",
            "error",
        ] {
            assert!(
                !PERMISSION_MESSAGE.contains(jargon),
                "the permission message mentions {jargon:?}, which means nothing to the user"
            );
        }
    }

    /// The backing struct handed to CoreAudio must have the layout the C
    /// variable-length idiom assumes: the count, then the buffers, with no
    /// padding surprises. If this ever fails, `copy_audio` is reading garbage.
    #[test]
    fn the_audio_buffer_list_backing_struct_matches_the_c_layout() {
        #[repr(C)]
        struct AudioBufferListN {
            number_buffers: u32,
            buffers: [AudioBuffer; MAX_CHANNELS],
        }

        // One buffer's worth of the fixed struct is exactly `AudioBufferList`.
        assert_eq!(
            std::mem::align_of::<AudioBufferListN>(),
            std::mem::align_of::<AudioBufferList>()
        );
        assert_eq!(
            std::mem::size_of::<AudioBufferListN>(),
            std::mem::size_of::<AudioBufferList>()
                + (MAX_CHANNELS - 1) * std::mem::size_of::<AudioBuffer>(),
            "the fixed-size backing struct is not laid out the way CoreAudio \
             expects a variable-length AudioBufferList"
        );
    }

    /// The trap that has already cost this project one CI round trip.
    ///
    /// `notetaker_core::capture::source::AudioSource` requires `Send`, and none
    /// of ScreenCaptureKit's objects are. The failure does not appear here — it
    /// appears in *core*, at the point the type is used as a `dyn AudioSource`,
    /// which is a crate that cannot be cross-compiled and so is only ever
    /// checked on a real Mac or in CI. That is exactly how `MicSource` shipped
    /// a `!Send` type to the first macOS build this project ever ran.
    ///
    /// Asserting it here moves the failure back to the crate that caused it,
    /// where a plain `cargo check` finds it in seconds. If this stops
    /// compiling, an Objective-C object has been stored in the struct — put it
    /// back on the worker thread rather than reaching for `unsafe impl Send`.
    #[test]
    fn the_source_is_send_because_core_requires_it() {
        fn assert_send<T: Send>() {}
        assert_send::<SystemAudioSource>();
    }
}
