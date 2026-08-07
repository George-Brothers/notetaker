//! The API layer the UI calls: plain, testable functions that a thin
//! `#[tauri::command]` wrapper (built separately, on the Mac side) will
//! delegate to one-to-one. Every type here serializes (`camelCase`) to
//! exactly the shape `src/lib/ipc.ts` expects.
//!
//! No globals, no singletons: every function takes the `Store`/`Index`/path
//! it needs as a parameter.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::index::Index;
use crate::storage::{Mode, RecordingRef, Status, Store};
use crate::watch::AutoRecordPolicy;

/// Sidecar file name, inside a recording's own directory, holding the AI's
/// task suggestion as plain text. `storage::Meta` has no field for this (it
/// is a transient suggestion awaiting a one-click accept/reject, not
/// durable recording metadata), so the pipeline writes it here instead.
/// Living inside the recording directory means it moves for free whenever
/// `Store::assign_task` renames that directory.
const SUGGESTED_TASK_FILE: &str = "suggested_task.txt";
/// Sidecar holding a better title than the timestamp the recording was created
/// with. Same reasoning as `SUGGESTED_TASK_FILE`: a transient offer awaiting a
/// one-click accept, not durable metadata.
const SUGGESTED_TITLE_FILE: &str = "suggested_title.txt";
const TRANSCRIPT_FILE: &str = "transcript.md";
const SUMMARY_FILE: &str = "summary.md";

/// The audio tracks a recording can have, as `<name>` in `audio-<name>.*`.
/// `system` only exists for a meeting; an in-person recording is mic only.
const TRACK_NAMES: &[&str] = &["mic", "system"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRow {
    pub id: String,
    pub title: String,
    pub task: Option<String>,
    pub created: String,
    pub duration_s: f64,
    pub mode: Mode,
    pub status: Status,
    pub suggested_task: Option<String>,
    /// A better title than the auto-generated timestamp, awaiting a one-click
    /// accept. On the row as well as the detail, so the library list can offer
    /// it without the user opening every recording.
    pub suggested_title: Option<String>,
    /// Why processing failed, so a failed row can explain itself in the list
    /// without the user having to open it.
    pub error: Option<String>,
    /// Why *capture* ended early or lost a track, if it did. Separate from
    /// `error` because it outlives every processing attempt: a `Ready` row can
    /// still need to say "this lecture is short because the disk filled up".
    pub capture_note: Option<String>,
    /// True if the user typed notes during or after this recording. Just the
    /// flag, not the text — the list shows a marker, and shipping every
    /// recording's full notes to render one icon would be wasteful.
    pub has_notes: bool,
    /// Archived recordings are kept separately from active work and excluded
    /// from search until restored.
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDetail {
    pub id: String,
    pub title: String,
    pub task: Option<String>,
    pub created: String,
    pub duration_s: f64,
    pub mode: Mode,
    pub status: Status,
    pub suggested_task: Option<String>,
    pub suggested_title: Option<String>,
    pub error: Option<String>,
    pub capture_note: Option<String>,
    pub transcript_md: String,
    pub summary_md: String,
    pub speakers: BTreeMap<String, String>,
    /// The user's own notes, verbatim. Never rewritten by the app — see the
    /// [`notes`](crate::notes) module note.
    pub notes_md: String,
    /// Which [template](crate::templates) shapes this recording's summary.
    /// `None` means the default.
    pub template: Option<String>,
    /// The checklist, parsed out of `summary_md`. Derived rather than stored, so
    /// it cannot disagree with the markdown the user can edit — see the
    /// [`actions`](crate::actions) module note.
    pub actions: Vec<crate::actions::ActionItem>,
    /// The transcript as timed segments, for the player. Empty for a recording
    /// that is unprocessed, or whose transcript the user has rewritten as prose;
    /// the UI then renders `transcript_md` directly.
    pub segments: Vec<crate::transcript::Segment>,
    /// Which audio tracks exist on disk (`"mic"`, `"system"`). The player offers
    /// only these — an in-person recording has no system track, and a meeting
    /// whose system track was lost must not present a control that plays
    /// silence.
    pub audio_tracks: Vec<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub task: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub storage_root: String,
    pub llm_base_url: String,
    pub llm_model: String,
    /// Optional Ollama model overrides keyed by task-folder name. An absent
    /// entry uses `llm_model` for that folder.
    #[serde(default)]
    pub task_models: BTreeMap<String, String>,
    pub tier_override: Option<String>,
    pub process_when_idle: bool,

    // --- Plan B. Every field below is `#[serde(default)]` so a settings file
    // written by Plan A still parses instead of resetting a user's config.
    /// What to do when each known meeting app appears, keyed by the watcher's
    /// app id (`"zoom"`). Apps absent from the map use
    /// [`AutoRecordPolicy::Ask`].
    #[serde(default)]
    pub auto_record: BTreeMap<String, AutoRecordPolicy>,
    /// Seconds of user inactivity before background processing may start.
    /// Ignored when `process_when_idle` is false.
    #[serde(default = "default_min_idle_secs")]
    pub min_idle_secs: u64,
    /// Only process while on wall power — the "and plugged in" half of Mr.
    /// Brothers' choice.
    #[serde(default = "default_true")]
    pub require_ac: bool,
    /// Keep the intermediate WAV after the FLAC finalize. Off by default: FLAC
    /// is lossless, so the WAV is pure duplication.
    #[serde(default)]
    pub keep_wav: bool,

    // --- Plan C: which languages this user actually speaks.
    /// The languages spoken in this user's recordings, as ISO-639-1 codes
    /// (`"en"`, `"zh"`, `"ja"`, `"ko"`, `"yue"`, …). First run asks; this is
    /// the answer.
    ///
    /// It exists to decide **what to download**. A 239 MB model that would
    /// never be chosen for your audio should not be fetched, and the only way
    /// to know that in advance is to ask. Defaults to English so an upgraded
    /// settings file behaves exactly as it did before.
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    /// Which speech model to use. `Auto` routes per segment; the other two
    /// force one model for everything.
    #[serde(default)]
    pub speech_engine: SpeechEngine,

    // --- 2026-08-04 UI overhaul. All defaulted so any older settings file
    // parses unchanged; see docs/superpowers/specs/2026-08-04-ui-overhaul-design.md §7.
    /// Which input device records. `None` means the system default.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Global accelerator that starts/stops recording, Tauri notation.
    #[serde(default = "default_hotkey_toggle_record")]
    pub hotkey_toggle_record: String,
    /// Global accelerator that shows/hides the window, Tauri notation.
    #[serde(default = "default_hotkey_show_hide")]
    pub hotkey_show_hide: String,
    /// Global accelerator that stars the current moment of a live recording.
    #[serde(default = "default_hotkey_highlight")]
    pub hotkey_highlight: String,
    /// Closing the window hides to the tray instead of quitting.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// When the floating meeting overlay appears. Desktop-shell-only — the
    /// served browser UI has no windows to float.
    #[serde(default)]
    pub overlay: OverlayMode,

    // --- Phase 6: Settings audit -----------------------------------------
    /// The preferred microphone order. The first available entry wins; an
    /// empty list means that the operating system's default is used.
    #[serde(default)]
    pub audio_device_priority: Vec<String>,
    /// High-level model performance preference. The runtime maps this onto
    /// the detected/forced model tier while `require_ac` remains the explicit
    /// battery gate for background work.
    #[serde(default)]
    pub performance_mode: PerformanceMode,
    /// How long speech models may remain resident after their last lease.
    /// Phase 1 owns the cache behavior; this field is also safe to read before
    /// that cache is present.
    #[serde(default)]
    pub model_idle_unload: ModelIdleUnload,
    /// Ollama model used by the dictation cleanup pass when that phase is
    /// enabled.
    #[serde(default = "default_cleanup_model")]
    pub cleanup_model: String,
    /// Whether the local cleanup pass should run for dictation.
    #[serde(default = "default_true")]
    pub dictation_cleanup_enabled: bool,
    /// Words and names that should be available to the dictation recognizer.
    #[serde(default)]
    pub dictation_dictionary: Vec<String>,
    /// Spoken form to corrected form, kept as a map for stable JSON and easy
    /// editing in Settings.
    #[serde(default)]
    pub dictation_replacements: BTreeMap<String, String>,
    /// The dictation interaction model. It is persisted now so the Phase 4
    /// workflow can use the same contract without another migration.
    #[serde(default)]
    pub dictation_mode: DictationMode,
    /// What happens after dictation text is produced.
    #[serde(default)]
    pub dictation_paste_behavior: PasteBehavior,
    /// Shortcut reserved for system-wide dictation.
    #[serde(default = "default_dictation_hotkey")]
    pub dictation_hotkey: String,
    /// Keep a lossless WAV copy of dictation audio in local history. Text
    /// history remains regardless of this switch; there is no auto-delete.
    #[serde(default)]
    pub dictation_keep_audio: bool,
    /// Where the overlay is placed when the desktop shell supports moving it.
    #[serde(default)]
    pub overlay_position: OverlayPosition,
    /// Visual treatment for the overlay.
    #[serde(default)]
    pub overlay_style: OverlayStyle,
    /// Whether the desktop shell should ask the OS to exclude the overlay
    /// from capture. macOS 15.4+ may still ignore that request.
    #[serde(default = "default_true")]
    pub overlay_hide_from_share: bool,
}

/// When the floating overlay (the little always-on-top recording pill) shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayMode {
    /// Never.
    Off,
    /// While a recording is running — it appears with the recording and
    /// leaves with it.
    #[default]
    Recording,
    /// From the moment a meeting is detected: doubles as the "record this?"
    /// prompt before anything is captured, then stays for the recording.
    Meeting,
}

/// Which speech model transcribes a recording.
///
/// The override exists because `Auto` cannot be right every time — a language
/// detector that mislabels a segment produces a wrong transcript with no
/// explanation, and without this the only remedy would be a code change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechEngine {
    /// Detect each segment's language and send it to the better model.
    #[default]
    Auto,
    /// Whisper for everything, whatever the language.
    Whisper,
    /// SenseVoice for everything. Falls back to Whisper if it is not
    /// downloaded, since refusing to transcribe would be worse than
    /// transcribing with the other model.
    SenseVoice,
}

/// Policy for releasing the native speech and speaker model set.
///
/// The wire values are deliberately short because this is persisted in
/// `settings.json` and exposed to the UI. `15s` exists only in debug builds so
/// the RAM acceptance run can finish quickly without making a production
/// setting that would surprise a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelIdleUnload {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "afterBatch")]
    AfterBatch,
    #[serde(rename = "2m")]
    TwoMinutes,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[cfg(debug_assertions)]
    #[serde(rename = "15s")]
    FifteenSeconds,
}

impl Default for ModelIdleUnload {
    fn default() -> Self {
        Self::FiveMinutes
    }
}

impl ModelIdleUnload {
    /// The idle interval used by the scheduler sweeper. `None` means never.
    /// `afterBatch` is represented by zero and is still checked only by the
    /// scheduler tick, so a lease can never be removed mid-job.
    pub fn idle_window(self) -> Option<Duration> {
        match self {
            Self::Never => None,
            Self::AfterBatch => Some(Duration::ZERO),
            Self::TwoMinutes => Some(Duration::from_secs(2 * 60)),
            Self::FiveMinutes => Some(Duration::from_secs(5 * 60)),
            Self::FifteenMinutes => Some(Duration::from_secs(15 * 60)),
            Self::OneHour => Some(Duration::from_secs(60 * 60)),
            #[cfg(debug_assertions)]
            Self::FifteenSeconds => Some(Duration::from_secs(15)),
        }
    }
}

/// High-level model preference shown in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PerformanceMode {
    /// Use the detected hardware tier and the user's battery policy.
    #[default]
    Auto,
    /// Prefer the largest tier this machine can use.
    BestQuality,
    /// Prefer the small tier to reduce CPU and memory pressure.
    CpuOptimized,
}

/// How dictation begins and ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationMode {
    /// Hold the shortcut while speaking.
    #[default]
    PushToTalk,
    /// Press once to start and again to finish.
    Toggle,
}

/// What the dictation workflow does with the resulting text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteBehavior {
    /// Insert text at the active cursor when permissions allow it.
    #[default]
    Paste,
    /// Leave text on the clipboard without sending a paste keystroke.
    CopyOnly,
}

/// Desktop overlay placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayPosition {
    #[default]
    TopRight,
    TopCenter,
    BottomCenter,
}

/// Desktop overlay skin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayStyle {
    #[default]
    Glass,
    Solid,
}

/// What the app can and cannot do right now.
///
/// Read from **disk and from the live scheduler**, never from what this session
/// happens to remember. The download checklist used to be the only thing that
/// knew whether setup had happened, and it is in-memory — so a restart made a
/// fully set-up app claim it had never started, and an app with no models at
/// all looked identical to a working one until you pressed something.
///
/// Nothing here blocks. The app records perfectly well with none of these
/// models; this exists so it can say so plainly instead of accepting work it
/// will silently never do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    /// Is the transcription loop actually running? When false, recordings are
    /// captured and kept, and nothing transcribes them.
    pub transcribing: bool,
    /// Required for this machine and these languages, and not on disk.
    pub missing: Vec<MissingModel>,
    /// What `missing` would cost to download, in bytes.
    pub download_bytes: u64,
    /// Recordings already waiting that this would unblock. The number that
    /// makes the difference between a notice and a nag.
    pub waiting: usize,
    /// Which hardware tier the model choice was made for, so the app can name
    /// it rather than presenting the size as arbitrary.
    pub tier: String,
}

/// A model candidate the user may choose to adopt instead of downloading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundModel {
    pub name: String,
    pub label: String,
}

/// One model the app needs and does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingModel {
    /// The identifier, matching `ModelSpec::name`.
    pub name: String,
    /// What it is, in words meant for a person.
    pub label: String,
    pub bytes: u64,
}

fn default_languages() -> Vec<String> {
    vec!["en".to_string()]
}

fn default_min_idle_secs() -> u64 {
    300
}

fn default_true() -> bool {
    true
}

fn default_hotkey_toggle_record() -> String {
    "CommandOrControl+Alt+N".to_string()
}

fn default_hotkey_show_hide() -> String {
    "CommandOrControl+Alt+Space".to_string()
}

fn default_hotkey_highlight() -> String {
    "CommandOrControl+Alt+H".to_string()
}

fn default_cleanup_model() -> String {
    "llama3.2:3b".to_string()
}

fn default_dictation_hotkey() -> String {
    "CommandOrControl+Alt+D".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            storage_root: String::new(),
            llm_base_url: "http://localhost:11434".to_string(),
            llm_model: "qwen3:8b".to_string(),
            task_models: BTreeMap::new(),
            tier_override: None,
            process_when_idle: true,
            auto_record: BTreeMap::new(),
            languages: default_languages(),
            speech_engine: SpeechEngine::Auto,
            min_idle_secs: default_min_idle_secs(),
            require_ac: default_true(),
            keep_wav: false,
            overlay: OverlayMode::default(),
            hotkey_highlight: default_hotkey_highlight(),
            input_device: None,
            hotkey_toggle_record: default_hotkey_toggle_record(),
            hotkey_show_hide: default_hotkey_show_hide(),
            close_to_tray: true,
            audio_device_priority: Vec::new(),
            performance_mode: PerformanceMode::Auto,
            model_idle_unload: ModelIdleUnload::default(),
            cleanup_model: default_cleanup_model(),
            dictation_cleanup_enabled: true,
            dictation_dictionary: Vec::new(),
            dictation_replacements: BTreeMap::new(),
            dictation_mode: DictationMode::PushToTalk,
            dictation_paste_behavior: PasteBehavior::Paste,
            dictation_hotkey: default_dictation_hotkey(),
            dictation_keep_audio: false,
            overlay_position: OverlayPosition::TopRight,
            overlay_style: OverlayStyle::Glass,
            overlay_hide_from_share: true,
        }
    }
}

/// Dir names under `Tasks/`.
pub fn list_tasks(store: &Store) -> Result<Vec<String>> {
    store.list_tasks()
}

pub fn create_task(store: &Store, name: &str) -> Result<()> {
    store.create_task(name)
}

/// Every recording on disk, newest first.
pub fn list_recordings(store: &Store) -> Result<Vec<RecordingRow>> {
    let mut recs = store.scan()?;
    recs.sort_by(|a, b| b.meta.created.cmp(&a.meta.created));
    Ok(recs.iter().map(to_row).collect())
}

/// Archived recordings, newest first. They retain every audio and note file,
/// but are intentionally absent from the active list.
pub fn list_archived_recordings(store: &Store) -> Result<Vec<RecordingRow>> {
    let mut recs = store.scan_archived()?;
    recs.sort_by(|a, b| b.meta.created.cmp(&a.meta.created));
    Ok(recs.iter().map(to_row).collect())
}

/// A single recording's full detail, including transcript/summary text. A
/// recording that hasn't been processed yet (no `transcript.md`/
/// `summary.md` on disk) returns empty strings for those, not an error.
pub fn get_recording(store: &Store, id: &str) -> Result<RecordingDetail> {
    let rec = find_by_id(store, id)?;
    let row = to_row(&rec);
    let transcript_md = fs::read_to_string(rec.dir.join(TRANSCRIPT_FILE)).unwrap_or_default();
    let summary_md = fs::read_to_string(rec.dir.join(SUMMARY_FILE)).unwrap_or_default();
    Ok(RecordingDetail {
        id: row.id,
        title: row.title,
        task: row.task,
        created: row.created,
        duration_s: row.duration_s,
        mode: row.mode,
        status: row.status,
        suggested_task: row.suggested_task,
        suggested_title: row.suggested_title,
        error: row.error,
        capture_note: row.capture_note,
        actions: crate::actions::parse(&summary_md),
        segments: crate::transcript::parse(&transcript_md, rec.meta.duration_s),
        transcript_md,
        summary_md,
        speakers: rec.meta.speakers.clone(),
        notes_md: crate::notes::read(&rec.dir),
        template: rec.meta.template.clone(),
        audio_tracks: audio_tracks(&rec.dir),
        archived: rec.archived,
    })
}

/// Saves the user's notes for a recording.
///
/// Allowed at any status, including while the recording is still running — that
/// is the whole point of a live notepad. Nothing here touches the audio or the
/// recording's folder, so it is safe during capture in a way that
/// `assign_task` and `rename_recording` deliberately are not.
pub fn save_notes(store: &Store, id: &str, notes_md: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    crate::notes::write(&rec.dir, notes_md)
}

/// Appends a jot from the floating overlay without rewriting `notes.md`.
pub fn append_note(store: &Store, id: &str, jot: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    crate::notes::append(&rec.dir, jot)?;
    Ok(())
}

/// Sets which template shapes this recording's summary.
///
/// Only changes the stored id; the summary is not rewritten until the recording
/// is processed again. The UI is responsible for saying so — silently leaving a
/// summary in the old shape after the user picked a new template would look
/// like the picker did nothing.
pub fn set_template(store: &Store, id: &str, template: &str) -> Result<()> {
    if !crate::templates::is_known(template) {
        anyhow::bail!("there is no note template called {template:?}");
    }
    let mut rec = find_by_id(store, id)?;
    rec.meta.template = Some(template.to_string());
    store.save_meta(&rec)
}

/// Ticks or unticks one action item, by rewriting that line of `summary.md`.
///
/// Returns the re-parsed checklist, so the caller never has to guess what the
/// list looks like afterwards — indices shift if the user has edited the
/// summary in the meantime, and a UI that assumed otherwise would tick the
/// wrong box.
pub fn set_action_done(
    store: &Store,
    id: &str,
    index: usize,
    done: bool,
) -> Result<Vec<crate::actions::ActionItem>> {
    let rec = find_by_id(store, id)?;
    let path = rec.dir.join(SUMMARY_FILE);
    let summary = fs::read_to_string(&path)
        .with_context(|| format!("reading the summary for {id} to tick an item"))?;
    let updated = crate::actions::set_done(&summary, index, done)?;
    fs::write(&path, &updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(crate::actions::parse(&updated))
}

/// The absolute path to one of a recording's audio tracks, for playback.
///
/// A path rather than the bytes: the desktop app hands it to the webview's own
/// file protocol and the served UI streams it with range requests, and neither
/// wants a whole lecture's audio marshalled through a JSON command.
pub fn audio_path(store: &Store, id: &str, track: &str) -> Result<std::path::PathBuf> {
    if !TRACK_NAMES.contains(&track) {
        anyhow::bail!("there is no audio track called {track:?}");
    }
    let rec = find_by_id(store, id)?;
    // FLAC first: it is what a finished recording has, and the WAV beside it is
    // a leftover the user chose to keep.
    for ext in ["flac", "wav"] {
        let p = rec.dir.join(format!("audio-{track}.{ext}"));
        if p.exists() {
            return Ok(p);
        }
    }
    anyhow::bail!("this recording has no {track} audio saved")
}

/// Persists a user's edit to the AI-written summary back to `summary.md`, so
/// the change survives closing the app and re-appears on next open.
pub fn update_summary(store: &Store, id: &str, summary_md: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    fs::write(rec.dir.join(SUMMARY_FILE), summary_md)
        .with_context(|| format!("writing summary for {id}"))?;
    Ok(())
}

pub fn search(index: &Index, query: &str) -> Result<Vec<SearchHit>> {
    Ok(index
        .search(query)?
        .into_iter()
        .map(|h| SearchHit {
            id: h.id,
            title: h.title,
            task: h.task,
            snippet: h.snippet,
        })
        .collect())
}

/// User-requested "process this now": unlike the idle-time queue's
/// `Queue::enqueue` (which deliberately leaves an already-`Ready` recording
/// alone during automatic sweeps), this is an explicit command and also
/// re-queues a `Ready` recording so the user can force a redo. `Queued`/
/// `Processing` are left alone — it's already in flight.
pub fn process_now(store: &Store, id: &str) -> Result<()> {
    let mut rec = find_by_id(store, id)?;
    match rec.meta.status {
        Status::Recorded | Status::Failed | Status::Ready => {
            rec.meta.status = Status::Queued;
            // `meta.error` describes a processing attempt and clears on retry
            // (ground rule) — without this, a recording that failed, was
            // retried by hand, and succeeded kept its old error text forever.
            rec.meta.error = None;
            rec.meta.manual_processing = true;
            store.save_meta(&rec)?;
        }
        Status::Queued => {
            // Queued work normally waits for the idle/power policy. A person
            // pressing the button is a different instruction: preserve it on
            // disk so a scheduler wake (or an app restart) cannot lose it.
            rec.meta.manual_processing = true;
            store.save_meta(&rec)?;
        }
        Status::Processing => {}
    }
    Ok(())
}

/// Retitles a recording, moving its folder to match.
///
/// Recordings are auto-titled ("Meeting — Jul 27, 2:30 PM") so that hitting
/// Record never blocks on typing a name; this is what makes that trade
/// survivable, because an auto-title is useless for finding a lecture three
/// weeks later.
///
/// Note this takes no `Index`: the search index still holds the old title
/// until the next `Index::rebuild` or a later `upsert` on this recording. The
/// app layer, which owns the `Index`, is where that refresh belongs.
pub fn rename_recording(store: &Store, id: &str, title: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    store.rename_recording(&rec, title)?;
    Ok(())
}

/// Moves the recording to `Tasks/<task>/` and re-indexes it there so a
/// search afterwards reports the new task.
pub fn assign_task(store: &Store, index: &mut Index, id: &str, task: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    let moved = store.assign_task(&rec, task)?;
    // The suggestion is resolved once the recording is filed; drop the
    // sidecar so the "Suggested: …" banner doesn't keep showing on a
    // recording that already lives in that task.
    let _ = fs::remove_file(moved.dir.join(SUGGESTED_TASK_FILE));
    index.upsert(&moved)?;
    Ok(())
}

/// Archives a recording and removes its searchable row. The files are moved,
/// not deleted, so Restore can put the exact same recording back.
pub fn archive_recording(store: &Store, index: &mut Index, id: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    store.archive_recording(&rec)?;
    index.remove(id)
}

pub fn restore_recording(store: &Store, index: &mut Index, id: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    let restored = store.restore_recording(&rec)?;
    index.upsert(&restored)
}

/// Permanently deletes one recording folder. The UI must show an explicit
/// confirmation first; this layer does not accept a path, only a known id.
pub fn delete_recording(store: &Store, index: &mut Index, id: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    store.delete_recording(&rec)?;
    index.remove(id)
}

/// Rewrites every occurrence of the speaker's current label (e.g.
/// `**Speaker 1:**`) to the new name (e.g. `**Jamie:**`) in `transcript.md`,
/// and records the mapping in `meta.speakers`.
///
/// The "current label" for `key` is whatever `meta.speakers` already has
/// for it (so a second rename — "Jamie" to "Sam" — replaces "Jamie", not
/// "Speaker 1"), falling back to the diarizer's default label (`"spk1"` ->
/// `"Speaker 1"`) the first time a key is renamed.
pub fn rename_speaker(store: &Store, id: &str, key: &str, name: &str) -> Result<()> {
    let mut rec = find_by_id(store, id)?;

    let old_label = rec
        .meta
        .speakers
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_label(key));

    let transcript_path = rec.dir.join(TRANSCRIPT_FILE);
    if let Ok(transcript) = fs::read_to_string(&transcript_path) {
        let old_tag = format!("**{old_label}:**");
        let new_tag = format!("**{name}:**");
        fs::write(&transcript_path, transcript.replace(&old_tag, &new_tag))
            .with_context(|| format!("writing {}", transcript_path.display()))?;
    }

    rec.meta.speakers.insert(key.to_string(), name.to_string());
    store.save_meta(&rec)
}

/// Reads settings as JSON from `path`. A missing file is not an error — it
/// means "never configured", so sensible defaults are returned instead.
pub fn get_settings(path: &Path) -> Result<Settings> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing settings at {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e).with_context(|| format!("reading settings at {}", path.display())),
    }
}

pub fn set_settings(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(path, json).with_context(|| format!("writing settings to {}", path.display()))
}

fn find_by_id(store: &Store, id: &str) -> Result<RecordingRef> {
    store
        .scan()?
        .into_iter()
        .chain(store.scan_archived()?)
        .find(|r| r.meta.id == id)
        .with_context(|| format!("no recording with id {id}"))
}

fn to_row(rec: &RecordingRef) -> RecordingRow {
    RecordingRow {
        id: rec.meta.id.clone(),
        title: rec.meta.title.clone(),
        task: rec.task.clone(),
        created: rec.meta.created.clone(),
        duration_s: rec.meta.duration_s,
        mode: rec.meta.mode,
        status: rec.meta.status,
        suggested_task: read_sidecar(&rec.dir, SUGGESTED_TASK_FILE),
        suggested_title: read_sidecar(&rec.dir, SUGGESTED_TITLE_FILE)
            // A suggestion identical to the current title is not a suggestion.
            // Happens after the user accepts one and the recording is later
            // reprocessed from the same summary.
            .filter(|t| t != &rec.meta.title),
        error: rec.meta.error.clone(),
        capture_note: rec.meta.capture_note.clone(),
        has_notes: crate::notes::has_content(&crate::notes::read(&rec.dir)),
        archived: rec.archived,
    }
}

/// Reads a one-line sidecar, treating missing, unreadable and blank alike as
/// "no suggestion". A suggestion is a nicety; none of its failure modes should
/// ever surface as an error to the user.
fn read_sidecar(dir: &Path, name: &str) -> Option<String> {
    let raw = fs::read_to_string(dir.join(name)).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Which audio tracks are actually on disk *and* have audio content, in
/// `TRACK_NAMES` order.
///
/// Existence is not enough. A meeting recorded with nothing playing through
/// the speakers leaves a WAV header with no audio frames — real, readable,
/// and completely silent. The WAV is 44 bytes of header, the FLAC is 4,469
/// bytes of valid compressed quiet. Both exist; only one is a track to offer.
///
/// For FLACs: a file that exists with bytes has audio (the encoder does not
/// create a file unless there is source audio to encode, and deletes it on
/// failure). For WAVs: open the file and check the frame count directly.
/// A damaged or unreadable file is not a track, and is not an error —
/// `audio_tracks` returns `Vec<String>`, not a `Result`; a broken file must
/// not take the whole detail fetch down.
fn audio_tracks(dir: &Path) -> Vec<String> {
    TRACK_NAMES
        .iter()
        .filter(|track| {
            // Try FLAC first: it is what a finished recording has.
            let flac_path = dir.join(format!("audio-{track}.flac"));
            if flac_path.exists() {
                if let Ok(metadata) = std::fs::metadata(&flac_path) {
                    if metadata.len() > 0 {
                        return true;
                    }
                }
            }

            // Then WAV: check for actual audio frames, not just a header.
            let wav_path = dir.join(format!("audio-{track}.wav"));
            if let Ok(reader) = hound::WavReader::open(&wav_path) {
                reader.len() > 0
            } else {
                false
            }
        })
        .map(|t| t.to_string())
        .collect()
}

/// The diarizer's default label for a speaker key: `"spk1"` -> `"Speaker
/// 1"`. Keys that don't match that pattern (e.g. `"George"` for the mic
/// track) are assumed to already equal their own transcript label.
fn default_label(key: &str) -> String {
    key.strip_prefix("spk")
        .filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        .map(|n| format!("Speaker {n}"))
        .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Mode;
    use chrono::TimeZone;

    fn store(dir: &Path) -> Store {
        Store::new(dir)
    }

    fn create(store: &Store, title: &str) -> RecordingRef {
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        store
            .create_recording(title, Mode::Meeting, created)
            .unwrap()
    }

    #[test]
    fn archive_restore_and_delete_update_lists_and_search_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Budget sync");
        fs::write(
            rec.dir.join("transcript.md"),
            "the quarterly budget is late",
        )
        .unwrap();
        fs::write(rec.dir.join("notes.md"), "Ask finance about the variance.").unwrap();
        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();
        assert_eq!(ix.search("budget").unwrap().len(), 1);

        archive_recording(&s, &mut ix, &rec.meta.id).unwrap();
        assert!(list_recordings(&s).unwrap().is_empty());
        let archived = list_archived_recordings(&s).unwrap();
        assert_eq!(archived.len(), 1);
        assert!(archived[0].archived);
        assert!(get_recording(&s, &rec.meta.id).unwrap().archived);
        assert!(ix.search("budget").unwrap().is_empty());

        restore_recording(&s, &mut ix, &rec.meta.id).unwrap();
        assert!(!get_recording(&s, &rec.meta.id).unwrap().archived);
        assert_eq!(ix.search("budget").unwrap().len(), 1);

        delete_recording(&s, &mut ix, &rec.meta.id).unwrap();
        assert!(list_recordings(&s).unwrap().is_empty());
        assert!(ix.search("budget").unwrap().is_empty());
        assert!(get_recording(&s, &rec.meta.id).is_err());
    }

    // --- rename_speaker -----------------------------------------------

    #[test]
    fn rename_speaker_rewrites_transcript_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");
        fs::write(
            rec.dir.join("transcript.md"),
            "[00:00:01] **Speaker 1:** hello\n[00:00:05] **Speaker 2:** hi back\n",
        )
        .unwrap();

        rename_speaker(&s, &rec.meta.id, "spk1", "Jamie").unwrap();

        let transcript = fs::read_to_string(rec.dir.join("transcript.md")).unwrap();
        assert!(transcript.contains("**Jamie:**"), "{transcript}");
        assert!(!transcript.contains("**Speaker 1:**"), "{transcript}");
        assert!(
            transcript.contains("**Speaker 2:**"),
            "unrelated speaker must be untouched: {transcript}"
        );

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(
            refreshed.meta.speakers.get("spk1"),
            Some(&"Jamie".to_string())
        );
    }

    #[test]
    fn rename_speaker_is_repeatable() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");
        fs::write(
            rec.dir.join("transcript.md"),
            "[00:00:01] **Speaker 1:** hello\n",
        )
        .unwrap();

        rename_speaker(&s, &rec.meta.id, "spk1", "Jamie").unwrap();
        rename_speaker(&s, &rec.meta.id, "spk1", "Sam").unwrap();

        let transcript = fs::read_to_string(rec.dir.join("transcript.md")).unwrap();
        assert!(transcript.contains("**Sam:**"), "{transcript}");
        assert!(!transcript.contains("**Jamie:**"), "{transcript}");
        assert!(!transcript.contains("**Speaker 1:**"), "{transcript}");

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(
            refreshed.meta.speakers.get("spk1"),
            Some(&"Sam".to_string())
        );
    }

    #[test]
    fn rename_speaker_with_no_transcript_yet_still_updates_meta() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");

        rename_speaker(&s, &rec.meta.id, "spk1", "Jamie").unwrap();

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(
            refreshed.meta.speakers.get("spk1"),
            Some(&"Jamie".to_string())
        );
    }

    // --- assign_task -----------------------------------------------------

    #[test]
    fn assign_task_moves_folder_and_search_reports_new_task() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Budget sync");
        fs::write(
            rec.dir.join("transcript.md"),
            "the quarterly budget is late",
        )
        .unwrap();

        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();
        assert_eq!(ix.search("budget").unwrap()[0].task, None);

        assign_task(&s, &mut ix, &rec.meta.id, "Accounting 302").unwrap();

        let hits = ix.search("budget").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].task.as_deref(), Some("Accounting 302"));

        let moved = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(moved.task.as_deref(), Some("Accounting 302"));
        assert!(moved
            .dir
            .starts_with(dir.path().join("Tasks").join("Accounting 302")));
    }

    // --- rename_recording ------------------------------------------------

    #[test]
    fn rename_recording_retitles_the_recording_and_moves_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Meeting — Aug 4, 10:02 AM");
        let old_dir = rec.dir.clone();

        rename_recording(&s, &rec.meta.id, "Accounting 302 midterm review").unwrap();

        assert!(!old_dir.exists());
        let detail = get_recording(&s, &rec.meta.id).unwrap();
        assert_eq!(detail.title, "Accounting 302 midterm review");
        let rows = list_recordings(&s).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Accounting 302 midterm review");
    }

    #[test]
    fn rename_recording_rejects_a_title_that_is_not_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Lecture 3");

        assert!(rename_recording(&s, &rec.meta.id, "  ").is_err());
        assert!(rename_recording(&s, &rec.meta.id, "..").is_err());
        assert!(rename_recording(&s, &rec.meta.id, "CS/Fall").is_err());
        assert_eq!(get_recording(&s, &rec.meta.id).unwrap().title, "Lecture 3");
    }

    #[test]
    fn rename_recording_unknown_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert!(rename_recording(&s, "nonexistent", "New name").is_err());
    }

    // --- capture_note ----------------------------------------------------

    #[test]
    fn capture_note_reaches_both_the_list_row_and_the_detail() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut rec = create(&s, "Lecture 3");
        rec.meta.capture_note =
            Some("Recording stopped because the disk was almost full.".to_string());
        s.save_meta(&rec).unwrap();

        let rows = list_recordings(&s).unwrap();
        assert_eq!(
            rows[0].capture_note.as_deref(),
            Some("Recording stopped because the disk was almost full.")
        );
        let detail = get_recording(&s, &rec.meta.id).unwrap();
        assert_eq!(
            detail.capture_note.as_deref(),
            Some("Recording stopped because the disk was almost full.")
        );
        assert_eq!(
            detail.error, None,
            "a capture note is not a processing error"
        );
    }

    // --- process_now -------------------------------------------------

    #[test]
    fn process_now_queues_recorded_failed_and_ready() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());

        let recorded = create(&s, "Recorded one");
        assert_eq!(recorded.meta.status, Status::Recorded);
        process_now(&s, &recorded.meta.id).unwrap();
        let queued = find_by_id(&s, &recorded.meta.id).unwrap();
        assert_eq!(queued.meta.status, Status::Queued);
        assert!(queued.meta.manual_processing);

        let mut failed = create(&s, "Failed one");
        failed.meta.status = Status::Failed;
        failed.meta.error = Some("diarization".to_string());
        s.save_meta(&failed).unwrap();
        process_now(&s, &failed.meta.id).unwrap();
        let requeued = find_by_id(&s, &failed.meta.id).unwrap();
        assert_eq!(requeued.meta.status, Status::Queued);
        assert_eq!(
            requeued.meta.error, None,
            "an error describes an attempt and clears on retry"
        );
        assert!(requeued.meta.manual_processing);

        let mut ready = create(&s, "Ready one");
        ready.meta.status = Status::Ready;
        s.save_meta(&ready).unwrap();
        process_now(&s, &ready.meta.id).unwrap();
        let requeued = find_by_id(&s, &ready.meta.id).unwrap();
        assert_eq!(requeued.meta.status, Status::Queued);
        assert!(requeued.meta.manual_processing);
    }

    #[test]
    fn process_now_marks_an_already_queued_recording_as_manual() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut queued = create(&s, "Queued one");
        queued.meta.status = Status::Queued;
        s.save_meta(&queued).unwrap();

        process_now(&s, &queued.meta.id).unwrap();
        assert!(find_by_id(&s, &queued.meta.id).unwrap().meta.manual_processing);
    }

    #[test]
    fn process_now_leaves_already_in_flight_recordings_alone() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut processing = create(&s, "Processing one");
        processing.meta.status = Status::Processing;
        s.save_meta(&processing).unwrap();

        process_now(&s, &processing.meta.id).unwrap();
        assert_eq!(
            find_by_id(&s, &processing.meta.id).unwrap().meta.status,
            Status::Processing
        );
    }

    // --- get_recording -----------------------------------------------

    #[test]
    fn get_recording_returns_empty_strings_when_no_transcript_or_summary() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Untouched");

        let detail = get_recording(&s, &rec.meta.id).unwrap();
        assert_eq!(detail.transcript_md, "");
        assert_eq!(detail.summary_md, "");
        assert_eq!(detail.id, rec.meta.id);
        assert_eq!(detail.error, None);
    }

    #[test]
    fn update_summary_persists_edited_text_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Lecture");
        fs::write(rec.dir.join("summary.md"), "## TL;DR\noriginal").unwrap();

        update_summary(&s, &rec.meta.id, "## TL;DR\nedited by hand").unwrap();

        let detail = get_recording(&s, &rec.meta.id).unwrap();
        assert_eq!(detail.summary_md, "## TL;DR\nedited by hand");
    }

    #[test]
    fn get_recording_returns_transcript_and_summary_text() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");
        fs::write(rec.dir.join("transcript.md"), "hello world").unwrap();
        fs::write(rec.dir.join("summary.md"), "## TL;DR\nhi").unwrap();

        let detail = get_recording(&s, &rec.meta.id).unwrap();
        assert_eq!(detail.transcript_md, "hello world");
        assert_eq!(detail.summary_md, "## TL;DR\nhi");
    }

    #[test]
    fn get_recording_unknown_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert!(get_recording(&s, "nonexistent").is_err());
    }

    // --- list_recordings / suggested_task sidecar ---------------------

    #[test]
    fn list_recordings_reads_suggested_task_sidecar_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");
        fs::write(rec.dir.join("suggested_task.txt"), "Accounting 302").unwrap();

        let rows = list_recordings(&s).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].suggested_task.as_deref(), Some("Accounting 302"));
    }

    #[test]
    fn list_recordings_suggested_task_is_none_without_sidecar_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        create(&s, "Standup");

        let rows = list_recordings(&s).unwrap();
        assert_eq!(rows[0].suggested_task, None);
    }

    // --- list_tasks / create_task --------------------------------------

    #[test]
    fn create_task_then_list_tasks_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert_eq!(list_tasks(&s).unwrap(), Vec::<String>::new());

        create_task(&s, "Accounting 302").unwrap();
        assert_eq!(list_tasks(&s).unwrap(), vec!["Accounting 302".to_string()]);
    }

    // --- search --------------------------------------------------------

    #[test]
    fn search_maps_index_hits_to_api_shape() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Budget sync");
        fs::write(
            rec.dir.join("transcript.md"),
            "the quarterly budget is late",
        )
        .unwrap();
        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        let hits = search(&ix, "budget").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, rec.meta.id);
        assert_eq!(hits[0].title, "Budget sync");
        assert_eq!(hits[0].task, None);
    }

    // --- settings --------------------------------------------------------

    #[test]
    fn get_settings_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");

        let settings = get_settings(&path).unwrap();
        assert_eq!(settings.llm_base_url, "http://localhost:11434");
        assert!(settings.process_when_idle);
    }

    #[test]
    fn set_settings_then_get_settings_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            storage_root: "/Users/george/Notetaker".to_string(),
            llm_base_url: "http://localhost:9999".to_string(),
            llm_model: "custom-model".to_string(),
            tier_override: Some("CpuSmall".to_string()),
            process_when_idle: false,
            auto_record: BTreeMap::from([
                ("zoom".to_string(), AutoRecordPolicy::Always),
                ("slack".to_string(), AutoRecordPolicy::Never),
            ]),
            min_idle_secs: 60,
            require_ac: false,
            keep_wav: true,
            languages: vec!["en".to_string(), "zh".to_string()],
            speech_engine: SpeechEngine::SenseVoice,
            overlay: OverlayMode::Meeting,
            hotkey_highlight: "CommandOrControl+Alt+B".to_string(),
            input_device: None,
            hotkey_toggle_record: "CommandOrControl+Alt+N".to_string(),
            hotkey_show_hide: "CommandOrControl+Alt+Space".to_string(),
            close_to_tray: false,
            ..Settings::default()
        };
        set_settings(&path, &settings).unwrap();

        let round_tripped = get_settings(&path).unwrap();
        assert_eq!(round_tripped, settings);
    }

    /// A Windows storage root must survive the round trip untouched.
    /// Backslashes are JSON escape characters, so a naive write-then-read can
    /// turn `C:\Users` into `C:Users` — and a drive letter's colon has broken
    /// more than one path parser. Both are silent: the app would come up
    /// pointing at a folder that isn't there and report an empty library
    /// rather than an error.
    #[test]
    fn windows_storage_root_round_trips_with_backslashes_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            storage_root: r"C:\Users\george\Notetaker".to_string(),
            tier_override: Some("CpuBig".to_string()),
            ..Settings::default()
        };
        set_settings(&path, &settings).unwrap();

        let loaded = get_settings(&path).unwrap();
        assert_eq!(loaded.storage_root, r"C:\Users\george\Notetaker");
        assert_eq!(loaded.tier_override.as_deref(), Some("CpuBig"));
    }

    /// A UNC path — `\\server\share` — is how a Windows user points the
    /// library at a NAS. The leading double backslash is the part that gets
    /// eaten by careless normalization.
    #[test]
    fn windows_unc_storage_root_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            storage_root: r"\\nas\media\Notetaker".to_string(),
            ..Settings::default()
        };
        set_settings(&path, &settings).unwrap();

        assert_eq!(
            get_settings(&path).unwrap().storage_root,
            r"\\nas\media\Notetaker"
        );
    }

    #[test]
    fn settings_written_before_plan_b_still_load() {
        // A settings file from Plan A has none of the capture/power fields. It
        // must keep working with its own values intact and defaults filled in
        // for the rest — an upgrade that silently reset a user's config would
        // be indistinguishable from data loss.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "storageRoot": "/Users/george/Notetaker",
                "llmBaseUrl": "http://localhost:11434",
                "llmModel": "qwen3:8b",
                "tierOverride": null,
                "processWhenIdle": false
            }"#,
        )
        .unwrap();

        let loaded = get_settings(&path).unwrap();
        assert_eq!(loaded.storage_root, "/Users/george/Notetaker");
        assert!(!loaded.process_when_idle, "existing value must survive");
        assert!(loaded.auto_record.is_empty());
        assert_eq!(loaded.min_idle_secs, 300);
        assert!(loaded.require_ac);
        assert!(!loaded.keep_wav);
        assert_eq!(loaded.model_idle_unload, ModelIdleUnload::FiveMinutes);
        assert_eq!(loaded.performance_mode, PerformanceMode::Auto);
        assert_eq!(loaded.dictation_hotkey, "CommandOrControl+Alt+D");
        assert!(loaded.overlay_hide_from_share);
    }

    #[test]
    fn set_settings_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("settings.json");
        set_settings(&path, &Settings::default()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn model_idle_unload_uses_the_persisted_wire_values_and_defaults_to_five_minutes() {
        assert_eq!(ModelIdleUnload::default(), ModelIdleUnload::FiveMinutes);
        let values = [
            (ModelIdleUnload::Never, "never"),
            (ModelIdleUnload::AfterBatch, "afterBatch"),
            (ModelIdleUnload::TwoMinutes, "2m"),
            (ModelIdleUnload::FiveMinutes, "5m"),
            (ModelIdleUnload::FifteenMinutes, "15m"),
            (ModelIdleUnload::OneHour, "1h"),
        ];
        for (policy, expected) in values {
            assert_eq!(serde_json::to_value(policy).unwrap(), expected);
        }
        #[cfg(debug_assertions)]
        assert_eq!(
            serde_json::to_value(ModelIdleUnload::FifteenSeconds).unwrap(),
            "15s"
        );

        let old = serde_json::json!({
            "storageRoot": "/tmp/Notetaker",
            "llmBaseUrl": "http://localhost:11434",
            "llmModel": "qwen3:8b",
            "tierOverride": null,
            "processWhenIdle": true
        });
        let loaded: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.model_idle_unload, ModelIdleUnload::FiveMinutes);
    }

    // --- audio_tracks ---------------------------------------------------

    #[test]
    fn audio_tracks_ignores_a_track_file_with_no_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        // What a meeting with nothing playing through the speakers leaves
        // behind: a mic track with audio, and a system track WASAPI never
        // wrote a sample to.
        std::fs::write(rec.dir.join("audio-mic.flac"), b"not empty").unwrap();
        std::fs::write(rec.dir.join("audio-system.flac"), b"").unwrap();

        assert_eq!(audio_tracks(&rec.dir), vec!["mic".to_string()]);
    }

    #[test]
    fn audio_tracks_lists_every_track_that_has_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        std::fs::write(rec.dir.join("audio-mic.flac"), b"not empty").unwrap();
        std::fs::write(rec.dir.join("audio-system.flac"), b"also not empty").unwrap();

        assert_eq!(
            audio_tracks(&rec.dir),
            vec!["mic".to_string(), "system".to_string()]
        );
    }

    #[test]
    fn audio_tracks_ignores_a_wav_that_captured_no_frames() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        // A WAV header with zero audio frames: WASAPI loopback wrote headers
        // but no samples because nothing was playing through the speakers.
        // This is the real shape of "the system track captured nothing".
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav_path = rec.dir.join("audio-system.wav");
        {
            let writer = hound::WavWriter::create(&wav_path, spec).unwrap();
            writer.finalize().unwrap();
        }
        // Sanity-check: the resulting file is around 44 bytes (WAV header only).
        let size = std::fs::metadata(&wav_path).unwrap().len();
        assert!(
            size > 40 && size < 50,
            "header-only WAV is ~44 bytes, got {size}"
        );

        // Add a mic track so the result is not empty (the test still works if
        // there is nothing left, but the message is clearer this way).
        std::fs::write(rec.dir.join("audio-mic.flac"), b"not empty").unwrap();

        assert_eq!(audio_tracks(&rec.dir), vec!["mic".to_string()]);
    }

    #[test]
    fn audio_tracks_lists_a_quiet_track_that_really_has_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        // A small but valid FLAC: the quiet audio from a meeting where only
        // one person was on the system track (everyone else on voice chat from
        // a desktop app, not Zoom). Quiet is not the same as absent; a track
        // that has audio content must be listed. This FLAC comes from the real
        // data: 4,469 bytes of compressed quiet.
        let small_flac = vec![
            0x66, 0x4C, 0x61, 0x43, 0x00, 0x00, 0x00, 0x22, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        std::fs::write(rec.dir.join("audio-system.flac"), &small_flac).unwrap();

        assert!(audio_tracks(&rec.dir).contains(&"system".to_string()));
    }

    #[test]
    fn audio_tracks_lists_a_track_once_when_both_flac_and_wav_survive() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        // Both mic.flac and mic.wav can survive on disk at once due to a known
        // bug in finalize_to_flac (file handle still open on Windows). The list
        // must still yield "mic" exactly once, not twice.
        std::fs::write(rec.dir.join("audio-mic.flac"), b"not empty").unwrap();

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(rec.dir.join("audio-mic.wav"), spec).unwrap();
            writer.write_sample(0i16).unwrap();
            writer.finalize().unwrap();
        }

        // Add a system track so we can also verify ordering (should be ["mic", "system"]).
        let small_flac = vec![
            0x66, 0x4C, 0x61, 0x43, 0x00, 0x00, 0x00, 0x22, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        std::fs::write(rec.dir.join("audio-system.flac"), &small_flac).unwrap();

        assert_eq!(
            audio_tracks(&rec.dir),
            vec!["mic".to_string(), "system".to_string()]
        );
    }

    #[test]
    fn audio_tracks_ignores_a_wav_it_cannot_even_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let rec = create(&store, "Team sync");

        // A genuinely corrupt file: garbage bytes, not a valid WAV at all.
        // `hound::WavReader::open()` will fail, and we should treat it as not
        // a track without panicking or propagating the error.
        std::fs::write(
            rec.dir.join("audio-mic.wav"),
            b"this is not a wav file at all",
        )
        .unwrap();

        // Add a system track so the result is not empty (the test still works
        // if there is nothing left, but the message is clearer this way).
        std::fs::write(rec.dir.join("audio-system.flac"), b"not empty").unwrap();

        // The corrupt WAV is not listed; the FLAC is.
        assert_eq!(audio_tracks(&rec.dir), vec!["system".to_string()]);
    }
}

#[cfg(test)]
mod overhaul_settings_tests {
    use super::*;

    /// A settings file written before the overhaul must parse, landing on the
    /// documented defaults instead of resetting the user's config.
    #[test]
    fn pre_overhaul_settings_json_gets_defaults() {
        let old = r#"{
            "storageRoot": "/tmp/x",
            "llmBaseUrl": "http://localhost:11434",
            "llmModel": "qwen3:8b",
            "tierOverride": null,
            "processWhenIdle": true
        }"#;
        let s: Settings = serde_json::from_str(old).expect("old settings must parse");
        assert_eq!(s.input_device, None);
        assert_eq!(s.hotkey_toggle_record, "CommandOrControl+Alt+N");
        assert_eq!(s.hotkey_show_hide, "CommandOrControl+Alt+Space");
        assert!(s.close_to_tray);
    }

    /// Round-trip: the new fields serialize camelCase, matching ipc.ts.
    #[test]
    fn new_fields_serialize_camel_case() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains("\"inputDevice\":null"));
        assert!(json.contains("\"hotkeyToggleRecord\":\"CommandOrControl+Alt+N\""));
        assert!(json.contains("\"hotkeyShowHide\":\"CommandOrControl+Alt+Space\""));
        assert!(json.contains("\"closeToTray\":true"));
        assert!(json.contains("\"modelIdleUnload\":\"5m\""));
        assert!(json.contains("\"performanceMode\":\"auto\""));
        assert!(json.contains("\"dictationHotkey\":\"CommandOrControl+Alt+D\""));
        assert!(json.contains("\"overlayHideFromShare\":true"));
    }
}
