//! Meeting watcher: noticing that a call started so the app can offer to
//! record it, per spec §4.2 ("detect meetings, ask first").
//!
//! Detection is process-based, which is portable — `sysinfo` enumerates
//! processes the same way on macOS and Linux — so this whole module is built
//! and tested here rather than waiting for the Mac.

pub mod apps;
pub mod mic;
pub mod watcher;

use serde::{Deserialize, Serialize};

/// What the app does when a known meeting app appears. Stored per app id in
/// `api::Settings::auto_record`, so "always record my Tuesday lecture Zoom"
/// is one click in the prompt rather than a settings expedition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoRecordPolicy {
    /// Show the "record this?" prompt. The default, per Mr. Brothers' choice
    /// of "detect meetings, ask first".
    #[default]
    Ask,
    /// Start recording without asking.
    Always,
    /// Ignore this app entirely — no prompt, no recording.
    Never,
}

/// A meeting app starting or stopping, after debouncing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingEvent {
    /// Stable id used as the `Settings::auto_record` key, e.g. `"zoom"`.
    pub app_id: String,
    /// Human name for the prompt, e.g. `"Zoom"`.
    pub app_name: String,
    pub kind: MeetingEventKind,
    /// True when policy is [`AutoRecordPolicy::Always`] — the UI starts
    /// recording instead of prompting.
    pub auto_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingEventKind {
    Started,
    Ended,
}
