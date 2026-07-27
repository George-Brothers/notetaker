//! Recording capture: turning live audio into the files the pipeline later
//! reads.
//!
//! The platform-bound part is deliberately tiny. Everything here — the session
//! state machine, incremental WAV writing, the disk guard, crash repair, FLAC
//! finalize — is portable and tested on Linux. The only macOS-specific code is
//! an [`AudioSource`] implementation (CoreAudio mic, ScreenCaptureKit system
//! audio), which lands in Plan B2 against this same trait.
//!
//! Track naming matches what `pipeline::run` already looks for: the stems
//! [`MIC_TRACK`] and [`SYSTEM_TRACK`], with either a `.wav` (mid-capture) or
//! `.flac` (finalized) extension.

pub mod flac;
pub mod recover;
pub mod session;
pub mod source;
pub mod track;

use serde::{Deserialize, Serialize};

use crate::storage::Mode;

/// File stem for the microphone track — the local user's own voice. Present in
/// both modes.
pub const MIC_TRACK: &str = "audio-mic";

/// File stem for the system-audio track — everyone else on the call. Meeting
/// mode only; in-person recordings have no system audio to capture.
pub const SYSTEM_TRACK: &str = "audio-system";

/// Capture sample rate. Matches what the pipeline requires end to end
/// (`pipeline::audio::load_mono_16k` rejects anything else), so there is no
/// resampling step anywhere in the app.
pub const SAMPLE_RATE: u32 = 16_000;

/// How often a track's buffered audio is forced to disk. A crash loses at most
/// this much of a recording.
pub const FLUSH_INTERVAL_SECS: u64 = 5;

/// Capture refuses to start, and stops an in-flight recording, below this much
/// free disk. Sized so stopping and finalizing still has room to work.
pub const MIN_FREE_MB: u64 = 500;

/// Where a capture session is in its lifecycle. Drives which controls the UI
/// enables, so it serializes to the UI in [`CaptureStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    /// Nothing being recorded.
    Idle,
    /// Actively consuming audio.
    Recording,
    /// Holding the files open, discarding incoming audio.
    Paused,
}

/// A live snapshot for the record bar: what state we're in, how long we've
/// been going, how loud each track is, and whether the disk is about to be a
/// problem. Polled by the UI; cheap to produce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub state: CaptureState,
    /// What kind of recording is running, `None` when nothing is.
    ///
    /// Carried in the snapshot rather than remembered by whichever component
    /// pressed Record, so that anything else polling capture — a menu bar, a
    /// window reopened mid-recording, a session recovered after a crash — can
    /// tell a meeting from an in-person lecture without having started it.
    pub mode: Option<Mode>,
    /// Id of the recording being captured, if any.
    pub recording_id: Option<String>,
    /// Seconds of audio actually captured — paused time is not counted.
    pub elapsed_s: f64,
    /// Peak level of the mic track since the last poll, 0.0..=1.0.
    pub mic_level: f32,
    /// Peak level of the system track since the last poll, 0.0..=1.0. Always
    /// 0.0 for in-person recordings, which have no system track.
    pub system_level: f32,
    /// Free space on the storage volume, for the UI's low-disk warning.
    pub disk_free_mb: u64,
}

/// Reports free space on the volume holding the recordings, so the disk guard
/// is a decision about a number rather than an untestable syscall.
pub trait DiskSpace: Send {
    /// Megabytes free where recordings are written, or `None` if the volume
    /// can't be read. An unreadable volume is treated as "no space" by the
    /// guard: refusing to start is recoverable, losing a lecture is not.
    fn free_mb(&self) -> Option<u64>;
}

/// A fixed disk reading, for tests that need to drive the guard.
pub struct FixedDisk(pub Option<u64>);

impl DiskSpace for FixedDisk {
    fn free_mb(&self) -> Option<u64> {
        self.0
    }
}

impl CaptureStatus {
    /// The status of a machine that isn't recording anything.
    pub fn idle(disk_free_mb: u64) -> Self {
        CaptureStatus {
            state: CaptureState::Idle,
            mode: None,
            recording_id: None,
            elapsed_s: 0.0,
            mic_level: 0.0,
            system_level: 0.0,
            disk_free_mb,
        }
    }
}
