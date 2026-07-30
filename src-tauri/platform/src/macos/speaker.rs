//! System audio on macOS, via ScreenCaptureKit. **Not yet implemented.**
//!
//! This file exists to hold a decision and a design, not a placeholder that
//! pretends to work. [`SystemAudioSource::start`] returns a clear error, which
//! is precisely how core's `CaptureSources::system` contract says a platform
//! reports "I cannot capture the other side of a call": **meeting mode then
//! refuses to start**, rather than silently recording half a conversation.
//!
//! So until this lands, a Mac can record in-person mode (microphone) but will
//! decline meeting mode with the message below. That is the existing Plan B1
//! behaviour, deliberately not changed here — recording a meeting that is
//! missing every other participant, and only finding out afterwards, is worse
//! than being told up front.
//!
//! # Why it is not written yet
//!
//! Everything else in this crate could be compile-verified from Linux against
//! the real target, which is what made writing it worthwhile before the
//! hardware arrived. This cannot be verified the same way, for two reasons that
//! do not apply anywhere else:
//!
//! 1. It needs an Objective-C **delegate class**, defined at runtime with
//!    `objc2::define_class!`, to receive `stream:didOutputSampleBuffer:ofType:`.
//!    Whether the selectors, protocol conformance and object lifetimes are
//!    right is exactly the kind of thing that compiles cleanly and then crashes
//!    or silently receives nothing.
//! 2. It needs the **Screen Recording permission**, which cannot be granted,
//!    denied, or revoked anywhere but on a real Mac in front of a real user —
//!    and the interesting paths are all in how the app behaves when it is
//!    refused.
//!
//! Writing it blind would produce code that looks finished, cannot be tested,
//! and would be indistinguishable from working code right up until a real
//! meeting was recorded silently. That is the one outcome worth avoiding.
//!
//! # The design, for the Mac day
//!
//! - `SCShareableContent::getShareableContentWithCompletionHandler` to
//!   enumerate displays. Any display will do; we want its audio, not its pixels.
//! - `SCContentFilter` over that display, then `SCStreamConfiguration` with
//!   `capturesAudio = true`, `excludesCurrentProcessAudio = true` (or Notetaker
//!   records its own notification sounds), `sampleRate = 48000`,
//!   `channelCount = 2`.
//! - Minimise the video side rather than disabling it: ScreenCaptureKit will
//!   not run without a video stream, so set `width`/`height` to something tiny
//!   and `minimumFrameInterval` to something long, then drop every video sample.
//! - A `define_class!` delegate implementing `SCStreamOutput`, adding audio
//!   sample buffers to [`crate::ring`] via [`crate::convert`], exactly as the
//!   Windows loopback path does. Everything below the delegate is already
//!   written, tested and shared.
//! - `CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer` to get PCM out of
//!   the `CMSampleBuffer`. ScreenCaptureKit delivers 48 kHz float, so
//!   [`crate::resample`] handles the rest — the 48 kHz -> 16 kHz path is already
//!   covered by tests.
//! - Permission: `SCShareableContent` fails if Screen Recording is not granted.
//!   That error must reach the user as plain English naming System Settings ->
//!   Privacy & Security -> Screen Recording, and the recording must continue
//!   with mic only rather than failing outright.
//!
//! The pieces this shares with Windows — ring buffer, downmix, resample — are
//! all tested. What is left is genuinely only the Apple-specific glue.

use anyhow::Result;

/// System audio on macOS. Not yet implemented; see the module docs.
pub struct SystemAudioSource;

impl SystemAudioSource {
    /// Always fails, with a message written for someone who is not an engineer.
    ///
    /// Per core's `CaptureSources::system` contract this makes meeting mode
    /// decline to start, so this message is what the user actually reads. It
    /// says what cannot be done and what still can, and it does not mention
    /// ScreenCaptureKit, a delegate, or a permission API.
    pub fn start() -> Result<Self> {
        anyhow::bail!(
            "Notetaker cannot record this computer's sound on a Mac yet, so it cannot \
             record a meeting — everyone else on the call would be missing. Recording \
             an in-person conversation with the microphone still works."
        )
    }

    pub fn read(&mut self, _out: &mut Vec<f32>) -> Result<()> {
        unreachable!("SystemAudioSource cannot be constructed on macOS yet")
    }

    pub fn is_finished(&self) -> bool {
        true
    }

    pub fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn label(&self) -> &str {
        "this computer's sound"
    }

    pub fn dropped_samples(&self) -> usize {
        0
    }
}
