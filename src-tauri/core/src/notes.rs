//! The user's own notes, typed live during a recording.
//!
//! This is the piece that makes the app a notepad rather than a recorder. The
//! user types rough notes while the meeting runs; the summarizer is then given
//! *both* the transcript and those notes, and asked to expand on what the user
//! bothered to write down rather than summarizing the call from scratch.
//!
//! Two separate files, deliberately:
//!
//! - `notes.md` — the user's text, verbatim, never rewritten by the app.
//! - `summary.md` — the AI's, which the user may edit.
//!
//! The alternative was one merged document with provenance markers in it, which
//! is what the UI *displays* (the user's words at full contrast, the AI's in
//! grey). But asking a language model to mark which sentences were the user's
//! means trusting it not to quietly reword them, and a notepad that edits your
//! own notes is unusable. Keeping the user's file untouched means the worst a
//! bad summarization can do is produce a bad summary next to your intact notes.
//!
//! Nothing here ever deletes: an empty save writes an empty file rather than
//! removing one, so "my notes vanished" cannot be caused by this module.

use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;
use std::path::Path;

use anyhow::{Context, Result};

/// The user's notes, inside the recording's own directory — so it moves for
/// free when `Store::assign_task` renames that directory, and travels with the
/// folder when it is copied to another machine.
pub const NOTES_FILE: &str = "notes.md";

/// Reads the user's notes. A recording with none returns an empty string, not
/// an error: most recordings never get typed notes, and that is not a fault.
pub fn read(dir: &Path) -> String {
    fs::read_to_string(dir.join(NOTES_FILE)).unwrap_or_default()
}

/// Writes the user's notes, replacing whatever was there.
pub fn write(dir: &Path, notes_md: &str) -> Result<()> {
    let path = dir.join(NOTES_FILE);
    fs::write(&path, notes_md).with_context(|| format!("writing your notes to {}", path.display()))
}

/// Appends a short jot without ever rewriting the existing notes file.
///
/// The expanded overlay is a second, small notepad surface. It must not use
/// [`write`] because a stale frontend snapshot could erase text typed in the
/// main window between keystrokes.
pub fn append(dir: &Path, jot: &str) -> Result<()> {
    let jot = jot.trim();
    if jot.is_empty() {
        return Ok(());
    }

    let path = dir.join(NOTES_FILE);
    let needs_separator = fs::read(&path)
        .map(|contents| !contents.is_empty() && !contents.ends_with(b"\n"))
        .unwrap_or(false);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening your notes for append at {}", path.display()))?;
    if needs_separator {
        file.write_all(b"\n")?;
    }
    file.write_all(jot.as_bytes())?;
    file.write_all(b"\n")
        .with_context(|| format!("appending your note to {}", path.display()))
}

/// Starred moments live in their own file, deliberately not in `notes.md`:
/// the notepad autosaves the whole notes file from its own editor state, so
/// anything a second writer appended there would be silently dropped by the
/// next keystroke's save. A file nothing else writes cannot lose a star.
pub const HIGHLIGHTS_FILE: &str = "highlights.md";

/// Appends one starred moment (`- ⭐ H:MM:SS`) and returns the appended line.
pub fn append_highlight(dir: &Path, elapsed_s: f64) -> Result<String> {
    let total = elapsed_s.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let line = format!("- ⭐ {h}:{m:02}:{s:02}");
    let path = dir.join(HIGHLIGHTS_FILE);
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&line);
    existing.push('\n');
    fs::write(&path, existing)
        .with_context(|| format!("writing your starred moment to {}", path.display()))?;
    Ok(line)
}

/// Reads the starred moments; a recording with none returns an empty string.
pub fn read_highlights(dir: &Path) -> String {
    fs::read_to_string(dir.join(HIGHLIGHTS_FILE)).unwrap_or_default()
}

/// True if the user actually typed something — whitespace does not count.
///
/// Drives whether the summarizer is told to expand on the user's notes or to
/// summarize the call cold, and whether the UI shows the notes pane or the
/// "start typing" placeholder.
pub fn has_content(notes_md: &str) -> bool {
    !notes_md.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "- ask about pricing\n- 15% is the number").unwrap();
        assert_eq!(read(dir.path()), "- ask about pricing\n- 15% is the number");
    }

    #[test]
    fn a_recording_with_no_notes_reads_as_empty_rather_than_erroring() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), "");
    }

    #[test]
    fn writing_replaces_rather_than_appends() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "first").unwrap();
        write(dir.path(), "second").unwrap();
        assert_eq!(read(dir.path()), "second");
    }

    /// Clearing the note box must leave a file behind, not remove one. A
    /// missing file and an empty file read the same here, but only one of them
    /// survives a folder sync that treats deletions as intentional.
    #[test]
    fn clearing_notes_writes_an_empty_file_rather_than_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "something").unwrap();
        write(dir.path(), "").unwrap();
        assert!(dir.path().join(NOTES_FILE).exists());
        assert_eq!(read(dir.path()), "");
    }

    #[test]
    fn cjk_and_newlines_survive() {
        let dir = tempfile::tempdir().unwrap();
        let notes = "线上会议\n- 价格 15%\n\n最后确认";
        write(dir.path(), notes).unwrap();
        assert_eq!(read(dir.path()), notes);
    }

    #[test]
    fn append_preserves_existing_notes_and_adds_a_line() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "first").unwrap();
        append(dir.path(), "second").unwrap();
        assert_eq!(read(dir.path()), "first\nsecond\n");
    }

    #[test]
    fn whitespace_is_not_content() {
        assert!(!has_content(""));
        assert!(!has_content("   \n\t\n  "));
        assert!(has_content("x"));
        assert!(has_content("\n  a note  \n"));
    }

    #[test]
    fn a_write_into_a_missing_directory_says_which_path_failed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-recording");
        let err = write(&missing, "x").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no-such-recording"),
            "error should name the path: {msg}"
        );
        assert!(
            msg.contains("your notes"),
            "error should be readable by a non-engineer: {msg}"
        );
    }
}
