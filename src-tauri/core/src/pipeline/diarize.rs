//! Speaker diarization: who spoke when.

/// A stretch of audio attributed to one (0-based, per-recording) speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeakerSpan {
    pub start_s: f32,
    pub end_s: f32,
    pub speaker: u32,
}
