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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::index::Index;
use crate::storage::{Mode, RecordingRef, Status, Store};

/// Sidecar file name, inside a recording's own directory, holding the AI's
/// task suggestion as plain text. `storage::Meta` has no field for this (it
/// is a transient suggestion awaiting a one-click accept/reject, not
/// durable recording metadata), so the pipeline writes it here instead.
/// Living inside the recording directory means it moves for free whenever
/// `Store::assign_task` renames that directory.
const SUGGESTED_TASK_FILE: &str = "suggested_task.txt";
const TRANSCRIPT_FILE: &str = "transcript.md";
const SUMMARY_FILE: &str = "summary.md";

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
    pub transcript_md: String,
    pub summary_md: String,
    pub speakers: BTreeMap<String, String>,
    pub error: Option<String>,
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
    pub tier_override: Option<String>,
    pub process_when_idle: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            storage_root: String::new(),
            llm_base_url: "http://localhost:11434".to_string(),
            llm_model: "qwen3:8b".to_string(),
            tier_override: None,
            process_when_idle: true,
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

/// A single recording's full detail, including transcript/summary text. A
/// recording that hasn't been processed yet (no `transcript.md`/
/// `summary.md` on disk) returns empty strings for those, not an error.
pub fn get_recording(store: &Store, id: &str) -> Result<RecordingDetail> {
    let rec = find_by_id(store, id)?;
    let row = to_row(&rec);
    Ok(RecordingDetail {
        id: row.id,
        title: row.title,
        task: row.task,
        created: row.created,
        duration_s: row.duration_s,
        mode: row.mode,
        status: row.status,
        suggested_task: row.suggested_task,
        transcript_md: fs::read_to_string(rec.dir.join(TRANSCRIPT_FILE)).unwrap_or_default(),
        summary_md: fs::read_to_string(rec.dir.join(SUMMARY_FILE)).unwrap_or_default(),
        speakers: rec.meta.speakers.clone(),
        error: rec.meta.error.clone(),
    })
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
            store.save_meta(&rec)?;
        }
        Status::Queued | Status::Processing => {}
    }
    Ok(())
}

/// Moves the recording to `Tasks/<task>/` and re-indexes it there so a
/// search afterwards reports the new task.
pub fn assign_task(store: &Store, index: &mut Index, id: &str, task: &str) -> Result<()> {
    let rec = find_by_id(store, id)?;
    let moved = store.assign_task(&rec, task)?;
    let transcript = fs::read_to_string(moved.dir.join(TRANSCRIPT_FILE)).unwrap_or_default();
    let summary = fs::read_to_string(moved.dir.join(SUMMARY_FILE)).unwrap_or_default();
    index.upsert(&moved, &transcript, &summary)?;
    Ok(())
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
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(path, json).with_context(|| format!("writing settings to {}", path.display()))
}

fn find_by_id(store: &Store, id: &str) -> Result<RecordingRef> {
    store
        .scan()?
        .into_iter()
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
        suggested_task: read_suggested_task(&rec.dir),
    }
}

fn read_suggested_task(dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(dir.join(SUGGESTED_TASK_FILE)).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        store.create_recording(title, Mode::Meeting, created).unwrap()
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
        assert!(transcript.contains("**Speaker 2:**"), "unrelated speaker must be untouched: {transcript}");

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(refreshed.meta.speakers.get("spk1"), Some(&"Jamie".to_string()));
    }

    #[test]
    fn rename_speaker_is_repeatable() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");
        fs::write(rec.dir.join("transcript.md"), "[00:00:01] **Speaker 1:** hello\n").unwrap();

        rename_speaker(&s, &rec.meta.id, "spk1", "Jamie").unwrap();
        rename_speaker(&s, &rec.meta.id, "spk1", "Sam").unwrap();

        let transcript = fs::read_to_string(rec.dir.join("transcript.md")).unwrap();
        assert!(transcript.contains("**Sam:**"), "{transcript}");
        assert!(!transcript.contains("**Jamie:**"), "{transcript}");
        assert!(!transcript.contains("**Speaker 1:**"), "{transcript}");

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(refreshed.meta.speakers.get("spk1"), Some(&"Sam".to_string()));
    }

    #[test]
    fn rename_speaker_with_no_transcript_yet_still_updates_meta() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Standup");

        rename_speaker(&s, &rec.meta.id, "spk1", "Jamie").unwrap();

        let refreshed = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(refreshed.meta.speakers.get("spk1"), Some(&"Jamie".to_string()));
    }

    // --- assign_task -----------------------------------------------------

    #[test]
    fn assign_task_moves_folder_and_search_reports_new_task() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let rec = create(&s, "Budget sync");
        fs::write(rec.dir.join("transcript.md"), "the quarterly budget is late").unwrap();

        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();
        assert_eq!(ix.search("budget").unwrap()[0].task, None);

        assign_task(&s, &mut ix, &rec.meta.id, "Accounting 302").unwrap();

        let hits = ix.search("budget").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].task.as_deref(), Some("Accounting 302"));

        let moved = find_by_id(&s, &rec.meta.id).unwrap();
        assert_eq!(moved.task.as_deref(), Some("Accounting 302"));
        assert!(moved.dir.starts_with(dir.path().join("Tasks").join("Accounting 302")));
    }

    // --- process_now -------------------------------------------------

    #[test]
    fn process_now_queues_recorded_failed_and_ready() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());

        let recorded = create(&s, "Recorded one");
        assert_eq!(recorded.meta.status, Status::Recorded);
        process_now(&s, &recorded.meta.id).unwrap();
        assert_eq!(find_by_id(&s, &recorded.meta.id).unwrap().meta.status, Status::Queued);

        let mut failed = create(&s, "Failed one");
        failed.meta.status = Status::Failed;
        s.save_meta(&failed).unwrap();
        process_now(&s, &failed.meta.id).unwrap();
        assert_eq!(find_by_id(&s, &failed.meta.id).unwrap().meta.status, Status::Queued);

        let mut ready = create(&s, "Ready one");
        ready.meta.status = Status::Ready;
        s.save_meta(&ready).unwrap();
        process_now(&s, &ready.meta.id).unwrap();
        assert_eq!(find_by_id(&s, &ready.meta.id).unwrap().meta.status, Status::Queued);
    }

    #[test]
    fn process_now_leaves_already_in_flight_recordings_alone() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let mut processing = create(&s, "Processing one");
        processing.meta.status = Status::Processing;
        s.save_meta(&processing).unwrap();

        process_now(&s, &processing.meta.id).unwrap();
        assert_eq!(find_by_id(&s, &processing.meta.id).unwrap().meta.status, Status::Processing);
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
        fs::write(rec.dir.join("transcript.md"), "the quarterly budget is late").unwrap();
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
        assert_eq!(settings.process_when_idle, true);
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
        };
        set_settings(&path, &settings).unwrap();

        let round_tripped = get_settings(&path).unwrap();
        assert_eq!(round_tripped, settings);
    }

    #[test]
    fn set_settings_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("settings.json");
        set_settings(&path, &Settings::default()).unwrap();
        assert!(path.exists());
    }
}
