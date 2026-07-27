//! Files-first storage layout: every recording is a folder on disk holding a
//! `meta.json`. Folders live under `<root>/Unsorted/` until assigned to a
//! task, at which point they move to `<root>/Tasks/<task>/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDateTime};
use serde::{Deserialize, Serialize};

const TASKS_DIR: &str = "Tasks";
const UNSORTED_DIR: &str = "Unsorted";
const META_FILE: &str = "meta.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Meeting,
    InPerson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Recorded,
    Queued,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String, // uuid v4
    pub title: String,
    pub mode: Mode,
    pub created: String, // RFC3339
    pub duration_s: f64,
    pub status: Status,
    pub speakers: BTreeMap<String, String>, // "spk1" -> display name
    /// Why *processing* failed. Deliberately transient: `Queue::enqueue`
    /// clears it on every retry, so a stale "download interrupted" never
    /// haunts a recording that is queued to run again.
    pub error: Option<String>,
    pub attempts: u32,
    /// Per-stage processing timings, filled by the pipeline. A real `Meta`
    /// field (not a loose JSON key) so it survives every `save_meta`.
    #[serde(default)]
    pub stages: Vec<StageTiming>,
    /// Why *capture* ended early or lost a track — a dead microphone, a full
    /// disk — in the plain English `Session::stop` wrote.
    ///
    /// A field of its own rather than a second use of `error`, because the two
    /// have opposite lifetimes. A processing error describes an attempt and is
    /// meant to be cleared by the next one; a capture problem describes the
    /// audio itself and is permanent — reprocessing a 20-minute lecture that
    /// should have been 40 cannot bring back the half the disk ate. Sharing
    /// `error` meant `Queue::enqueue` wiped the explanation the moment the
    /// recording was queued, and the user never learned why it was short.
    ///
    /// `#[serde(default)]` so `meta.json` files written before this field
    /// existed still parse.
    #[serde(default)]
    pub capture_note: Option<String>,
}

/// How long one pipeline stage took, for diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageTiming {
    pub stage: String,
    pub ms: u64,
}

#[derive(Debug, Clone)]
pub struct RecordingRef {
    pub meta: Meta,
    pub dir: PathBuf,
    pub task: Option<String>, // task None = Unsorted
}

pub struct Store {
    pub root: PathBuf,
}

/// Strip filesystem-hostile characters and trim whitespace so a title is
/// safe to use as a directory name.
fn sanitize(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '/' | ':' | '\\' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Read and parse the `meta.json` inside `dir`, deriving the enclosing
/// `RecordingRef` (task = parent dir name if under `Tasks/`, else `None`).
fn load_ref(dir: &Path, task: Option<String>) -> Result<RecordingRef> {
    let raw = fs::read_to_string(dir.join(META_FILE))?;
    let meta: Meta = serde_json::from_str(&raw)?;
    Ok(RecordingRef {
        meta,
        dir: dir.to_path_buf(),
        task,
    })
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// Creates `Unsorted/<YYYY-MM-DD HH.MM title>/meta.json` with status
    /// `Recorded` and returns the new `RecordingRef`.
    pub fn create_recording(
        &self,
        title: &str,
        mode: Mode,
        created: DateTime<Local>,
    ) -> Result<RecordingRef> {
        let unsorted = self.root.join(UNSORTED_DIR);
        fs::create_dir_all(&unsorted)
            .with_context(|| format!("creating {}", unsorted.display()))?;

        let stamp = created.format("%Y-%m-%d %H.%M");
        let base_name = format!("{} {}", stamp, sanitize(title));
        let dir = unique_dir(&unsorted, &base_name);
        fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let meta = Meta {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            mode,
            created: created.to_rfc3339(),
            duration_s: 0.0,
            status: Status::Recorded,
            speakers: BTreeMap::new(),
            error: None,
            attempts: 0,
            stages: Vec::new(),
            capture_note: None,
        };

        let rec = RecordingRef {
            meta,
            dir,
            task: None,
        };
        self.save_meta(&rec)?;
        Ok(rec)
    }

    /// Writes `rec.meta` to `<rec.dir>/meta.json`.
    pub fn save_meta(&self, rec: &RecordingRef) -> Result<()> {
        let path = rec.dir.join(META_FILE);
        let json = serde_json::to_string_pretty(&rec.meta)?;
        fs::write(&path, json).with_context(|| format!("writing {}", path.display()))
    }

    /// Walks `Tasks/*/*` and `Unsorted/*`, parsing every `meta.json` found.
    /// Unparseable recording dirs are logged and skipped, never abort the
    /// whole scan.
    pub fn scan(&self) -> Result<Vec<RecordingRef>> {
        let mut out = Vec::new();

        let unsorted = self.root.join(UNSORTED_DIR);
        for dir in list_subdirs(&unsorted)? {
            match load_ref(&dir, None) {
                Ok(r) => out.push(r),
                Err(e) => log::warn!("skipping unreadable recording {}: {e}", dir.display()),
            }
        }

        let tasks = self.root.join(TASKS_DIR);
        for task_dir in list_subdirs(&tasks)? {
            let task_name = task_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for dir in list_subdirs(&task_dir)? {
                match load_ref(&dir, Some(task_name.clone())) {
                    Ok(r) => out.push(r),
                    Err(e) => log::warn!("skipping unreadable recording {}: {e}", dir.display()),
                }
            }
        }

        Ok(out)
    }

    /// Dir names under `Tasks/`.
    pub fn list_tasks(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = list_subdirs(&self.root.join(TASKS_DIR))?
            .into_iter()
            .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn create_task(&self, name: &str) -> Result<()> {
        let name = safe_task_name(name)?;
        let dir = self.root.join(TASKS_DIR).join(&name);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))
    }

    /// Moves the recording's folder into `Tasks/<task>/`, then re-derives
    /// its `RecordingRef` from the new location.
    pub fn assign_task(&self, rec: &RecordingRef, task: &str) -> Result<RecordingRef> {
        let task = safe_task_name(task)?;
        let task_dir = self.root.join(TASKS_DIR).join(&task);
        fs::create_dir_all(&task_dir)
            .with_context(|| format!("creating {}", task_dir.display()))?;

        let file_name = rec
            .dir
            .file_name()
            .context("recording dir has no file name")?;
        let dest = unique_dir(&task_dir, &file_name.to_string_lossy());
        fs::rename(&rec.dir, &dest)
            .with_context(|| format!("moving {} to {}", rec.dir.display(), dest.display()))?;

        load_ref(&dest, Some(task))
    }

    /// Retitles a recording: rewrites `meta.title` and renames the folder to
    /// match, keeping its `<YYYY-MM-DD HH.MM …>` prefix and its place in the
    /// tree (an unsorted recording stays unsorted, a filed one stays in its
    /// task).
    ///
    /// Renaming has to move the directory because the title *is* part of the
    /// directory name, and the on-disk layout is the public contract — a
    /// recording the user renamed but whose folder still says "Meeting — Jul
    /// 27" would be unfindable in Finder, which is half the point of a
    /// files-first layout. Modelled on [`Store::assign_task`], down to
    /// re-deriving the `RecordingRef` from the new location.
    pub fn rename_recording(&self, rec: &RecordingRef, title: &str) -> Result<RecordingRef> {
        let folder_title = safe_recording_title(title)?;
        let parent = rec.dir.parent().context("recording dir has no parent")?;
        let base = match dir_stamp(&rec.dir) {
            Some(stamp) => format!("{stamp} {folder_title}"),
            None => folder_title,
        };

        let mut renamed = rec.clone();
        renamed.meta.title = title.trim().to_string();

        // Renaming a recording to what it is already called must not turn its
        // folder into "… (2)" — `unique_dir` would count the recording's own
        // directory as the collision.
        if parent.join(&base) != rec.dir {
            let dest = unique_dir(parent, &base);
            fs::rename(&rec.dir, &dest)
                .with_context(|| format!("moving {} to {}", rec.dir.display(), dest.display()))?;
            renamed.dir = dest;
        }

        self.save_meta(&renamed)?;
        load_ref(&renamed.dir, rec.task.clone())
    }

    /// Re-reads a recording's `meta.json` from disk. Used after a stage has
    /// enriched the file (e.g. the pipeline writing `speakers`/`stages`) so a
    /// caller holding a pre-enrichment copy doesn't overwrite it.
    pub fn reload(&self, rec: &RecordingRef) -> Result<RecordingRef> {
        load_ref(&rec.dir, rec.task.clone())
    }
}

/// Validates a user-supplied task name and returns a filesystem-safe form.
/// Rejects names that would escape or restructure the `Tasks/` tree — the
/// on-disk layout is a public contract and `scan()` only walks one level
/// deep, so a `/` in a name would hide every recording filed under it.
fn safe_task_name(name: &str) -> Result<String> {
    let cleaned = sanitize(name);
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        anyhow::bail!("invalid task name: {name:?}");
    }
    Ok(cleaned)
}

/// Validates a user-supplied recording title and returns the form used for
/// the folder name.
///
/// Empty, `.` and `..` are rejected for the same reason `safe_task_name`
/// rejects them: they are paths, not names. A separator is rejected outright
/// rather than flattened the way a task name is — a task name is typed once to
/// create a folder, but a title is the user's own words on their own
/// recording, and silently turning "Lecture 3/4" into "Lecture 34" edits what
/// they wrote. Better to say no and let them retype it.
fn safe_recording_title(title: &str) -> Result<String> {
    if title.contains('/') || title.contains('\\') {
        anyhow::bail!("recording title must not contain a path separator: {title:?}");
    }
    let cleaned = sanitize(title);
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        anyhow::bail!("invalid recording title: {title:?}");
    }
    Ok(cleaned)
}

/// The `YYYY-MM-DD HH.MM` prefix a recording's folder carries, if it has one.
///
/// Read off the folder itself rather than re-derived from `meta.created`, so a
/// rename can never re-time a recording, and a `created` field that no longer
/// parses can never make one impossible to rename.
fn dir_stamp(dir: &Path) -> Option<String> {
    /// The `%Y-%m-%d %H.%M` that `create_recording` writes.
    const STAMP_FORMAT: &str = "%Y-%m-%d %H.%M";
    /// Width of a rendered stamp: "2026-08-04 10.02".
    const STAMP_LEN: usize = 16;

    let name = dir.file_name()?.to_string_lossy().into_owned();
    let stamp = name.get(..STAMP_LEN)?;
    NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT).ok()?;
    Some(stamp.to_string())
}

/// List immediate subdirectories of `parent`. An absent `parent` yields an
/// empty list rather than an error (scan tolerates a store with no
/// recordings yet).
fn list_subdirs(parent: &Path) -> Result<Vec<PathBuf>> {
    if !parent.is_dir() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(parent).with_context(|| format!("reading {}", parent.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

/// Returns `parent/name`, or `parent/name (2)`, `parent/name (3)`, ... if
/// that path already exists.
fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let candidate = parent.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let mut n = 2;
    loop {
        let candidate = parent.join(format!("{name} ({n})"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn create_lands_in_unsorted_with_meta() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::InPerson, created).unwrap();
        assert!(r.dir.ends_with("Unsorted/2026-08-04 10.02 Lecture 3"));
        assert!(r.dir.join("meta.json").exists());
        assert!(matches!(r.meta.status, Status::Recorded));
        assert!(r.task.is_none());
    }

    #[test]
    fn scan_roundtrips_and_assign_moves() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        s.create_task("Accounting 302").unwrap();
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::Meeting, created).unwrap();
        let r = s.assign_task(&r, "Accounting 302").unwrap();
        assert_eq!(r.task.as_deref(), Some("Accounting 302"));
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta.title, "Lecture 3");
        assert_eq!(s.list_tasks().unwrap(), vec!["Accounting 302"]);
    }

    #[test]
    fn task_names_with_separators_are_flattened_and_dotdot_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();

        // A "/" in a task name must not create a nested Tasks/CS/Fall that
        // scan() (one level deep) would miss — the recording must stay findable.
        let r = s.create_recording("Lecture", Mode::InPerson, created).unwrap();
        let moved = s.assign_task(&r, "CS/Fall").unwrap();
        assert_eq!(moved.task.as_deref(), Some("CSFall"));
        assert!(moved.dir.starts_with(dir.path().join("Tasks")));
        assert_eq!(s.scan().unwrap().len(), 1, "recording must remain findable");

        // Traversal attempts are rejected outright.
        assert!(s.create_task("..").is_err());
        assert!(s.create_task("").is_err());
    }

    // --- rename_recording -------------------------------------------------

    #[test]
    fn rename_moves_the_folder_updates_the_title_and_keeps_the_recording_findable() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s
            .create_recording("Meeting — Aug 4, 10:02 AM", Mode::Meeting, created)
            .unwrap();
        let old_dir = r.dir.clone();
        let id = r.meta.id.clone();

        let renamed = s.rename_recording(&r, "Accounting 302 midterm review").unwrap();

        assert!(!old_dir.exists(), "the old folder must not be left behind");
        assert!(
            renamed
                .dir
                .ends_with("Unsorted/2026-08-04 10.02 Accounting 302 midterm review"),
            "{}",
            renamed.dir.display()
        );
        assert_eq!(renamed.meta.title, "Accounting 302 midterm review");
        assert!(renamed.dir.join(META_FILE).exists());

        // Still exactly one recording, still the same one, still findable by id.
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta.id, id);
        assert_eq!(all[0].meta.title, "Accounting 302 midterm review");
    }

    #[test]
    fn rename_rejects_titles_that_are_not_names_without_touching_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::InPerson, created).unwrap();

        for bad in ["", "   ", ".", "..", "CS/Fall", "..\\escape"] {
            assert!(
                s.rename_recording(&r, bad).is_err(),
                "{bad:?} must be rejected"
            );
            assert!(r.dir.exists(), "a rejected rename must not move anything");
            let all = s.scan().unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].meta.title, "Lecture 3", "title must be untouched");
        }
    }

    #[test]
    fn rename_onto_a_colliding_folder_name_destroys_neither_recording() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let keep = s.create_recording("Taken", Mode::InPerson, created).unwrap();
        let rename_me = s.create_recording("Other", Mode::InPerson, created).unwrap();

        // Same timestamp prefix + same title = the same folder name.
        let renamed = s.rename_recording(&rename_me, "Taken").unwrap();

        assert!(keep.dir.exists(), "the existing recording must survive");
        assert_ne!(renamed.dir, keep.dir);
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 2, "both recordings must still be on disk");
        assert!(all.iter().any(|r| r.meta.id == keep.meta.id));
        assert!(all.iter().any(|r| r.meta.id == rename_me.meta.id));
    }

    #[test]
    fn renaming_a_recording_filed_under_a_task_keeps_it_in_that_task() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::InPerson, created).unwrap();
        let filed = s.assign_task(&r, "Accounting 302").unwrap();

        let renamed = s.rename_recording(&filed, "Depreciation deep dive").unwrap();

        assert_eq!(renamed.task.as_deref(), Some("Accounting 302"));
        assert!(renamed
            .dir
            .starts_with(dir.path().join(TASKS_DIR).join("Accounting 302")));
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta.title, "Depreciation deep dive");
        assert_eq!(all[0].task.as_deref(), Some("Accounting 302"));
    }

    #[test]
    fn renaming_to_the_same_title_is_a_no_op_rather_than_a_numbered_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::InPerson, created).unwrap();

        let renamed = s.rename_recording(&r, "Lecture 3").unwrap();

        assert_eq!(renamed.dir, r.dir, "must not become 'Lecture 3 (2)'");
        assert_eq!(s.scan().unwrap().len(), 1);
    }

    // --- capture_note ------------------------------------------------------

    #[test]
    fn capture_note_round_trips_through_save_and_scan() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let mut r = s.create_recording("Lecture", Mode::InPerson, created).unwrap();
        assert_eq!(r.meta.capture_note, None, "a clean capture leaves no note");

        r.meta.capture_note = Some("The microphone stopped working.".to_string());
        s.save_meta(&r).unwrap();

        assert_eq!(
            s.reload(&r).unwrap().meta.capture_note.as_deref(),
            Some("The microphone stopped working.")
        );
    }

    #[test]
    fn meta_written_before_capture_notes_still_parses() {
        // A meta.json from before this field existed must keep loading with its
        // own values intact — a recording that stopped parsing on upgrade is
        // indistinguishable from a lecture that vanished.
        let dir = tempfile::tempdir().unwrap();
        let rec_dir = dir.path().join(UNSORTED_DIR).join("2026-08-04 10.02 Lecture");
        fs::create_dir_all(&rec_dir).unwrap();
        fs::write(
            rec_dir.join(META_FILE),
            r#"{
                "id": "abc-123",
                "title": "Lecture",
                "mode": "in_person",
                "created": "2026-08-04T10:02:00-05:00",
                "duration_s": 12.5,
                "status": "ready",
                "speakers": {},
                "error": null,
                "attempts": 0
            }"#,
        )
        .unwrap();

        let s = Store::new(dir.path());
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 1, "an older meta.json must still be readable");
        assert_eq!(all[0].meta.title, "Lecture");
        assert_eq!(all[0].meta.duration_s, 12.5);
        assert_eq!(all[0].meta.capture_note, None);
        assert!(all[0].meta.stages.is_empty());
    }

    #[test]
    fn stages_field_round_trips_through_save_and_scan() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let mut r = s.create_recording("Lecture", Mode::InPerson, created).unwrap();
        r.meta.stages.push(StageTiming {
            stage: "transcribe".to_string(),
            ms: 42,
        });
        s.save_meta(&r).unwrap();

        let reloaded = s.reload(&r).unwrap();
        assert_eq!(reloaded.meta.stages.len(), 1);
        assert_eq!(reloaded.meta.stages[0].ms, 42);
    }
}
