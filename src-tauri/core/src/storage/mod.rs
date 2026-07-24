//! Files-first storage layout: every recording is a folder on disk holding a
//! `meta.json`. Folders live under `<root>/Unsorted/` until assigned to a
//! task, at which point they move to `<root>/Tasks/<task>/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
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
    pub error: Option<String>,
    pub attempts: u32,
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
        let dir = self.root.join(TASKS_DIR).join(name);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))
    }

    /// Moves the recording's folder into `Tasks/<task>/`, then re-derives
    /// its `RecordingRef` from the new location.
    pub fn assign_task(&self, rec: &RecordingRef, task: &str) -> Result<RecordingRef> {
        let task_dir = self.root.join(TASKS_DIR).join(task);
        fs::create_dir_all(&task_dir)
            .with_context(|| format!("creating {}", task_dir.display()))?;

        let file_name = rec
            .dir
            .file_name()
            .context("recording dir has no file name")?;
        let dest = unique_dir(&task_dir, &file_name.to_string_lossy());
        fs::rename(&rec.dir, &dest)
            .with_context(|| format!("moving {} to {}", rec.dir.display(), dest.display()))?;

        load_ref(&dest, Some(task.to_string()))
    }
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
}
