//! Wiring the real per-OS capture devices onto this crate's traits.
//!
//! `notetaker-platform` deliberately knows nothing about this crate — that is
//! what keeps its dependency tree pure Rust and lets it be type-checked against
//! macOS and Windows from any machine. The cost of that choice is paid here, in
//! one small adapter: core owns the traits, so core writes the `impl`s.
//!
//! There is no logic in this file. Every method forwards. If something in here
//! grows a decision, it belongs on one side of the boundary or the other.

use anyhow::Result;

use super::source::AudioSource;
use crate::runtime::CaptureSources;

/// The real devices on this machine.
///
/// Replaces `runtime::FakeSources` in the shipped app. Constructing one does
/// nothing — devices are opened per recording by [`CaptureSources::mic`] and
/// [`CaptureSources::system`], because a microphone held open between
/// recordings is a microphone the user sees a red light for.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformSources;

impl PlatformSources {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl CaptureSources for PlatformSources {
    fn mic(&self) -> Result<Box<dyn AudioSource>> {
        Ok(Box::new(notetaker_platform::MicSource::start()?))
    }

    fn system(&self) -> Result<Box<dyn AudioSource>> {
        // Both platforms can now capture the other side of a call: WASAPI
        // loopback on Windows, ScreenCaptureKit on macOS. An error from either
        // still means what the `CaptureSources::system` contract says it means
        // — meeting mode declines to start rather than recording half a
        // conversation — but on macOS it now describes a *refused permission*
        // rather than an unimplemented platform, and the message says how to
        // grant it.
        #[cfg(target_os = "windows")]
        {
            Ok(Box::new(
                notetaker_platform::windows::SystemAudioSource::start()?,
            ))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Box::new(
                notetaker_platform::macos::speaker::SystemAudioSource::start()?,
            ))
        }
    }
}

/// Linux has no capture implementation — `cpal` is not built there (its
/// `alsa-sys` needs pkg-config unavailable in this build environment), and
/// Linux is a build host for this project rather than a target. Both methods
/// therefore fail with a message that says so instead of pretending.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl CaptureSources for PlatformSources {
    fn mic(&self) -> Result<Box<dyn AudioSource>> {
        anyhow::bail!("Notetaker can only record on macOS and Windows.")
    }

    fn system(&self) -> Result<Box<dyn AudioSource>> {
        anyhow::bail!("Notetaker can only record on macOS and Windows.")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl AudioSource for notetaker_platform::MicSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        notetaker_platform::MicSource::read(self, out)
    }

    fn is_finished(&self) -> bool {
        notetaker_platform::MicSource::is_finished(self)
    }

    fn stop(&mut self) -> Result<()> {
        notetaker_platform::MicSource::stop(self)
    }

    fn label(&self) -> &str {
        notetaker_platform::MicSource::label(self)
    }
}

#[cfg(target_os = "windows")]
impl AudioSource for notetaker_platform::windows::SystemAudioSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        notetaker_platform::windows::SystemAudioSource::read(self, out)
    }

    fn is_finished(&self) -> bool {
        notetaker_platform::windows::SystemAudioSource::is_finished(self)
    }

    fn stop(&mut self) -> Result<()> {
        notetaker_platform::windows::SystemAudioSource::stop(self)
    }

    fn label(&self) -> &str {
        notetaker_platform::windows::SystemAudioSource::label(self)
    }
}

#[cfg(target_os = "macos")]
impl AudioSource for notetaker_platform::macos::speaker::SystemAudioSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        notetaker_platform::macos::speaker::SystemAudioSource::read(self, out)
    }

    fn is_finished(&self) -> bool {
        notetaker_platform::macos::speaker::SystemAudioSource::is_finished(self)
    }

    fn stop(&mut self) -> Result<()> {
        notetaker_platform::macos::speaker::SystemAudioSource::stop(self)
    }

    fn label(&self) -> &str {
        notetaker_platform::macos::speaker::SystemAudioSource::label(self)
    }
}

#[cfg(test)]
mod tests {
    /// The one thing this boundary can get wrong silently.
    ///
    /// `notetaker_platform::TARGET_SAMPLE_RATE` is duplicated rather than
    /// imported from here, because the platform crate must not depend on this
    /// one. Duplicated constants drift, and this particular drift would be
    /// invisible: every capture source would resample to the wrong rate,
    /// `pipeline::audio::load_mono_16k` would reject every recording, and the
    /// symptom would be "processing always fails" with nothing pointing here.
    ///
    /// This test is why the duplication is safe. It runs on every platform,
    /// including Linux where no capture code is built at all.
    #[test]
    fn the_platform_crate_resamples_to_our_capture_sample_rate() {
        assert_eq!(
            notetaker_platform::TARGET_SAMPLE_RATE,
            crate::capture::SAMPLE_RATE,
            "notetaker-platform resamples to a different rate than capture expects; \
             every recording would be rejected by the pipeline"
        );
    }
}
