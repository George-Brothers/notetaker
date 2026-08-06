//! Non-prompting macOS TCC probes used by the first-run checklist.
//!
//! A missing grant is not an error. The app must distinguish "not granted"
//! from "the probe itself failed", so every function here returns a boolean
//! and never requests access or opens System Settings.

use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_core_graphics::CGPreflightListenEventAccess;

use crate::PermissionStatus;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

/// Reads the current microphone, Accessibility, and Input Monitoring state.
///
/// Input Monitoring is deliberately reported but not required by the current
/// implementation: the global shortcut plugin uses Carbon, and paste uses a
/// posted CGEvent. If a future feature installs an event tap, it can flip the
/// `input_monitoring_required` bit and the onboarding UI will expose the same
/// exact probe without treating an unrelated false value as a failure.
pub fn read() -> PermissionStatus {
    let microphone = unsafe {
        AVMediaTypeAudio
            .as_ref()
            .map(|media_type| {
                AVCaptureDevice::authorizationStatusForMediaType(media_type)
                    == AVAuthorizationStatus::Authorized
            })
            .unwrap_or(false)
    };

    PermissionStatus {
        microphone,
        accessibility: unsafe { AXIsProcessTrusted() != 0 },
        input_monitoring: CGPreflightListenEventAccess(),
        input_monitoring_required: false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn input_monitoring_is_not_required_by_current_dictation_path() {
        // This test documents the deliberate boundary: Carbon registers the
        // hotkey and CGEvent posts Cmd-V; neither installs an event tap.
        assert!(!super::read().input_monitoring_required);
    }
}
