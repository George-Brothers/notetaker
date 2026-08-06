//! Windows has no corresponding TCC gates for the shipped dictation path.

use crate::PermissionStatus;

pub fn read() -> PermissionStatus {
    PermissionStatus {
        microphone: true,
        accessibility: true,
        input_monitoring: true,
        input_monitoring_required: false,
    }
}
