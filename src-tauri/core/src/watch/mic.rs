//! The real mic-in-use probe, wired onto the watcher's [`MicSource`] seam.
//!
//! Same shape as `capture::platform`: `notetaker-platform` knows nothing about
//! this crate, so core owns the trait and writes the one forwarding impl. On
//! an OS with no probe (Linux — the dev machine) the answer is always "quiet",
//! which simply disables mic detection rather than faking one.

use super::watcher::MicSource;

/// The default microphone's in-use state, asked of the OS on every poll.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformMic;

impl MicSource for PlatformMic {
    #[cfg(target_os = "macos")]
    fn mic_in_use(&self) -> bool {
        // "Could not ask" is a quiet mic, not a meeting.
        notetaker_platform::macos::mic_activity::mic_in_use().unwrap_or(false)
    }

    #[cfg(target_os = "windows")]
    fn mic_in_use(&self) -> bool {
        notetaker_platform::windows::mic_activity::mic_in_use().unwrap_or(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn mic_in_use(&self) -> bool {
        false
    }
}
