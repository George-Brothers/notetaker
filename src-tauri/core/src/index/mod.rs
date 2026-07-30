//! SQLite FTS5 search index over transcripts and summaries.
//!
//! Disposable cache: every row is derived from the plain files under a
//! `Store`'s root, so the whole database can be deleted and rebuilt with
//! `rebuild`. Supports English and Chinese search — see `segment_cjk` for
//! why Chinese needs special handling with the default FTS5 tokenizer.

use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::storage::{Mode, RecordingRef, Status, Store};

pub struct Index {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub task: Option<String>,
    pub snippet: String,
}

/// FTS5's default `unicode61` tokenizer treats a whole run of Han
/// characters as a single token, so a search for a 2-character word like
/// `预算` inside a longer run of Chinese text never matches. Inserting a
/// space around every CJK codepoint (on both index and query sides) turns
/// each character into its own token, which `unicode61` handles correctly.
fn segment_cjk(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF) {
            out.push(' ');
            out.push(ch);
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Reverses `segment_cjk`'s spacing in text pulled back out of the index
/// (snippets), so the UI never shows `预 算 讨 论`. A space is dropped
/// whenever the character before or after it (skipping markup inserted by
/// `snippet()`) is CJK, since those are the spaces we inserted.
fn desegment_cjk(s: &str) -> String {
    fn is_cjk(c: char) -> bool {
        matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
    }

    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' {
            let prev_cjk = out.chars().last().is_some_and(is_cjk);
            let next_cjk = chars.get(i + 1).is_some_and(|&c| is_cjk(c));
            if prev_cjk || next_cjk {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Builds an FTS5 MATCH expression from a user's query.
///
/// Each whitespace-separated word becomes its own quoted phrase and the
/// phrases are ANDed, so "budget hiring" finds a transcript containing both
/// words anywhere — not only next to each other. Quoting each phrase means
/// FTS5 operator characters the user typed (`OR`, `-`, `*`, ...) are literal
/// text rather than syntax; a literal `"` is escaped by doubling so it can
/// never break out of its phrase.
///
/// A CJK word stays one phrase after segmentation: `预算` indexes as `预 算`
/// and must match as the phrase `"预 算"`, not as two independent characters,
/// or it would also match any text containing both characters far apart.
fn fts_phrase(query: &str) -> String {
    let phrases: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            let escaped = segment_cjk(word).replace('"', "\"\"");
            format!("\"{}\"", escaped.trim())
        })
        .filter(|p| p != "\"\"")
        .collect();

    if phrases.is_empty() {
        // An all-whitespace or empty query matches nothing rather than
        // erroring out on an empty MATCH expression.
        return "\"\"".to_string();
    }
    phrases.join(" AND ")
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Meeting => "meeting",
        Mode::InPerson => "in_person",
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Recorded => "recorded",
        Status::Queued => "queued",
        Status::Processing => "processing",
        Status::Ready => "ready",
        Status::Failed => "failed",
    }
}

/// The searchable columns, in order. `snippet()` addresses columns by index,
/// so `notes` is appended rather than inserted — putting it before `summary`
/// would silently change which column the snippet comes from.
const FTS_COLUMNS: &[&str] = &["id", "title", "transcript", "summary", "notes"];

/// The column `search` draws its snippet from: `transcript`, at index 2.
const SNIPPET_COLUMN: usize = 2;

/// Fallback snippet column when a recording has no transcript yet: `notes`, at
/// index 4. A recording the user typed notes into but has not processed is
/// findable, and an empty snippet on the result would read as a broken row.
const SNIPPET_FALLBACK_COLUMN: usize = 4;

/// True if `recordings_fts` already has exactly [`FTS_COLUMNS`].
///
/// A table that does not exist counts as matching: the `CREATE ... IF NOT
/// EXISTS` that follows will make it correctly, and reporting a mismatch would
/// mean issuing a pointless `DROP` on every first run.
fn fts_schema_matches(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(recordings_fts)")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    if names.is_empty() {
        return Ok(true);
    }
    Ok(names == FTS_COLUMNS)
}

/// Reads one of a recording's text files, treating "not there" as empty.
///
/// Every file this reads is legitimately absent for part of a recording's life
/// — no `transcript.md` before processing, no `notes.md` if the user never
/// typed — so a missing file is normal, not an error to propagate.
fn read_text(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

impl Index {
    pub fn open(db_path: &Path) -> Result<Index> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recordings(
                id TEXT PRIMARY KEY, title TEXT, task TEXT, created TEXT,
                duration_s REAL, mode TEXT, status TEXT, dir TEXT
            );",
        )?;

        // `CREATE VIRTUAL TABLE IF NOT EXISTS` will not *alter* a table that
        // already exists with different columns, so a database written by an
        // older build would keep its old shape and every insert would fail on
        // the column count. This index is a disposable cache — `Runtime::
        // start_up` rebuilds it on every launch — so the correct migration is
        // to throw the table away and let that rebuild refill it.
        if !fts_schema_matches(&conn)? {
            conn.execute_batch("DROP TABLE IF EXISTS recordings_fts;")?;
        }
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS recordings_fts
                USING fts5(id UNINDEXED, {});",
            FTS_COLUMNS[1..].join(", ")
        ))?;

        Ok(Index { conn })
    }

    /// Full rescan of `store`: clears both tables and re-indexes every
    /// recording found on disk. Returns the number of recordings indexed.
    pub fn rebuild(&mut self, store: &Store) -> Result<usize> {
        let recs = store.scan()?;

        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM recordings", [])?;
        tx.execute("DELETE FROM recordings_fts", [])?;
        tx.commit()?;

        for rec in &recs {
            self.upsert(rec)?;
        }
        Ok(recs.len())
    }

    /// Indexes (or re-indexes) a single recording, reading its text straight
    /// from the recording's own directory.
    ///
    /// Reading here rather than taking the text as arguments means a caller
    /// cannot index something other than what is on disk — every caller was
    /// already reading these three files immediately beforehand, and the
    /// version that took strings made "write the file, then index the old
    /// text" an easy mistake to make. Files that do not exist (an unprocessed
    /// recording, a recording with no typed notes) index as empty.
    pub fn upsert(&mut self, rec: &RecordingRef) -> Result<()> {
        let id = &rec.meta.id;
        let transcript = read_text(&rec.dir, "transcript.md");
        let summary = read_text(&rec.dir, "summary.md");
        let notes = read_text(&rec.dir, crate::notes::NOTES_FILE);

        self.conn.execute(
            "INSERT INTO recordings(id, title, task, created, duration_s, mode, status, dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title, task = excluded.task, created = excluded.created,
                duration_s = excluded.duration_s, mode = excluded.mode,
                status = excluded.status, dir = excluded.dir",
            params![
                id,
                rec.meta.title,
                rec.task,
                rec.meta.created,
                rec.meta.duration_s,
                mode_str(rec.meta.mode),
                status_str(rec.meta.status),
                rec.dir.to_string_lossy(),
            ],
        )?;

        // fts5 has no upsert; drop the old row (if any) and re-insert.
        self.conn
            .execute("DELETE FROM recordings_fts WHERE id = ?1", params![id])?;
        self.conn.execute(
            "INSERT INTO recordings_fts(id, title, transcript, summary, notes)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                segment_cjk(&rec.meta.title),
                segment_cjk(&transcript),
                segment_cjk(&summary),
                segment_cjk(&notes),
            ],
        )?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let match_expr = fts_phrase(query);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT r.id, r.title, r.task,
                    snippet(recordings_fts, {SNIPPET_COLUMN}, '<b>', '</b>', '…', 12) AS snip,
                    snippet(recordings_fts, {SNIPPET_FALLBACK_COLUMN}, '<b>', '</b>', '…', 12) AS notes_snip
             FROM recordings_fts
             JOIN recordings r ON r.id = recordings_fts.id
             WHERE recordings_fts MATCH ?1
             ORDER BY rank"
        ))?;
        let rows = stmt.query_map(params![match_expr], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let task: Option<String> = row.get(2)?;
            let snip: String = row.get(3)?;
            let notes_snip: String = row.get(4)?;
            // A recording with notes but no transcript yet is still findable,
            // and would otherwise show a blank row.
            let best = if snip.trim().is_empty() {
                notes_snip
            } else {
                snip
            };
            Ok(SearchHit {
                id,
                title,
                task,
                snippet: desegment_cjk(&best),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Mode;
    use chrono::TimeZone;

    #[test]
    fn multi_word_query_matches_words_that_are_not_adjacent() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(
            r.dir.join("transcript.md"),
            "George: the budget is late\nSpeaker 1: and the hiring plan slipped too",
        )
        .unwrap();
        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        // The words are lines apart; a phrase-only query would find nothing.
        assert_eq!(ix.search("budget hiring").unwrap().len(), 1);
        // A word that is absent must still exclude the recording.
        assert_eq!(ix.search("budget parking").unwrap().len(), 0);
    }

    #[test]
    fn cjk_word_stays_one_phrase_and_is_not_split_into_loose_characters() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Lecture", Mode::InPerson, created)
            .unwrap();
        // Contains 预 and 算 far apart, but never the word 预算.
        std::fs::write(
            r.dir.join("transcript.md"),
            "Speaker 1: 预计的方案还要再算一次",
        )
        .unwrap();
        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        assert_eq!(ix.search("预算").unwrap().len(), 0);
    }

    #[test]
    fn rebuild_indexes_transcripts_and_survives_db_delete() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(
            r.dir.join("transcript.md"),
            "George: the quarterly budget is late",
        )
        .unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        assert_eq!(ix.rebuild(&s).unwrap(), 1);
        assert_eq!(
            ix.search("quarterly budget").unwrap()[0].title,
            "Budget sync"
        );
        drop(ix);
        std::fs::remove_file(&db).unwrap(); // index is disposable
        let mut ix2 = Index::open(&db).unwrap();
        ix2.rebuild(&s).unwrap();
        assert_eq!(ix2.search("quarterly").unwrap().len(), 1); // proves rebuildability
    }

    #[test]
    fn indexes_recordings_without_transcript_or_summary() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        s.create_recording("Untouched lecture", Mode::InPerson, created)
            .unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        assert_eq!(ix.rebuild(&s).unwrap(), 1);
        let hits = ix.search("Untouched").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Untouched lecture");
    }

    #[test]
    fn finds_two_character_chinese_term_and_english_term_in_bilingual_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Planning sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(
            r.dir.join("transcript.md"),
            "We reviewed the quarterly budget today. 我们今天讨论预算和招聘计划",
        )
        .unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        ix.rebuild(&s).unwrap();

        let cn_hits = ix.search("预算").unwrap();
        assert_eq!(cn_hits.len(), 1, "two-character Chinese term should match");
        assert_eq!(cn_hits[0].title, "Planning sync");

        let cn_hits2 = ix.search("招聘计划").unwrap();
        assert_eq!(
            cn_hits2.len(),
            1,
            "four-character Chinese phrase should match"
        );

        let en_hits = ix.search("quarterly").unwrap();
        assert_eq!(
            en_hits.len(),
            1,
            "English term should still match in the same bilingual corpus"
        );
    }

    #[test]
    fn snippet_strips_inserted_cjk_spacing() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Planning sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(r.dir.join("transcript.md"), "我们今天讨论预算和招聘计划").unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        ix.rebuild(&s).unwrap();

        let hits = ix.search("预算").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("预算"),
            "snippet should contain natural, unsegmented Chinese text, was: {:?}",
            hits[0].snippet
        );
        assert!(
            !hits[0].snippet.contains("预 算"),
            "snippet still shows inserted CJK spacing: {:?}",
            hits[0].snippet
        );
    }

    #[test]
    fn search_handles_quotes_and_operators_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(r.dir.join("transcript.md"), "the quarterly budget is late").unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        ix.rebuild(&s).unwrap();

        let result = ix.search("budget OR \"");
        assert!(
            result.is_ok(),
            "a query containing quote/operator characters must never produce a SQL/FTS syntax error: {:?}",
            result.err()
        );
    }

    #[test]
    fn upsert_reindexes_without_duplicating_rows() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();

        std::fs::write(r.dir.join("transcript.md"), "first draft transcript").unwrap();
        ix.upsert(&r).unwrap();
        std::fs::write(
            r.dir.join("transcript.md"),
            "revised transcript about pricing",
        )
        .unwrap();
        ix.upsert(&r).unwrap();

        assert_eq!(ix.search("pricing").unwrap().len(), 1);
        assert_eq!(ix.search("first draft").unwrap().len(), 0);
    }

    // --- the user's own notes are searchable -----------------------------

    /// Notes are often the only place a phrase exists — you write "chase the
    /// Henderson invoice" and nobody says the word "Henderson" out loud. If
    /// notes were not indexed, searching for what you typed would find nothing.
    #[test]
    fn a_phrase_only_in_the_users_notes_is_findable() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(r.dir.join("transcript.md"), "the quarterly budget is late").unwrap();
        crate::notes::write(&r.dir, "- chase the Henderson invoice").unwrap();

        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        let hits = ix.search("Henderson").unwrap();
        assert_eq!(
            hits.len(),
            1,
            "a phrase from the user's notes was not found"
        );
        assert_eq!(hits[0].title, "Budget sync");
    }

    /// A recording typed into but never processed has no transcript, so the
    /// transcript snippet is blank. The row must still explain itself.
    #[test]
    fn a_notes_only_recording_gets_its_snippet_from_the_notes() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Cold call", Mode::Meeting, created)
            .unwrap();
        crate::notes::write(&r.dir, "- they want net 30 terms").unwrap();

        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        let hits = ix.search("net 30").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("net"),
            "an unprocessed recording showed a blank snippet: {:?}",
            hits[0].snippet
        );
    }

    #[test]
    fn chinese_notes_are_searchable_and_desegmented() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s.create_recording("讨论", Mode::Meeting, created).unwrap();
        crate::notes::write(&r.dir, "预算还没定").unwrap();

        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        let hits = ix.search("预算").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].snippet.contains("预 算"), "{:?}", hits[0].snippet);
    }

    // --- schema migration ------------------------------------------------

    /// The failure this prevents: a database written before `notes` existed
    /// keeps its four-column table, `CREATE ... IF NOT EXISTS` leaves it alone,
    /// and every insert then fails on the column count — so search silently
    /// returns nothing for the whole library until the file is deleted by hand.
    #[test]
    fn a_database_from_before_notes_existed_is_rebuilt_not_broken() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ix.sqlite");

        // Exactly the old schema.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS recordings(
                    id TEXT PRIMARY KEY, title TEXT, task TEXT, created TEXT,
                    duration_s REAL, mode TEXT, status TEXT, dir TEXT
                );
                CREATE VIRTUAL TABLE IF NOT EXISTS recordings_fts
                    USING fts5(id UNINDEXED, title, transcript, summary);",
            )
            .unwrap();
        }

        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(r.dir.join("transcript.md"), "the budget is late").unwrap();
        crate::notes::write(&r.dir, "- Henderson invoice").unwrap();

        // Opening must migrate; the startup rebuild then refills it.
        let mut ix = Index::open(&db).unwrap();
        assert_eq!(ix.rebuild(&s).unwrap(), 1);
        assert_eq!(ix.search("budget").unwrap().len(), 1);
        assert_eq!(
            ix.search("Henderson").unwrap().len(),
            1,
            "the migrated table is not indexing notes"
        );
    }

    /// Reopening a current database must not drop and rebuild it — that would
    /// throw the index away on every single launch.
    #[test]
    fn reopening_a_current_database_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ix.sqlite");
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let r = s
            .create_recording("Budget sync", Mode::Meeting, created)
            .unwrap();
        std::fs::write(r.dir.join("transcript.md"), "the budget is late").unwrap();

        {
            let mut ix = Index::open(&db).unwrap();
            ix.rebuild(&s).unwrap();
            assert_eq!(ix.search("budget").unwrap().len(), 1);
        }

        let reopened = Index::open(&db).unwrap();
        assert_eq!(
            reopened.search("budget").unwrap().len(),
            1,
            "reopening dropped the index"
        );
    }

    #[test]
    fn the_snippet_column_constants_address_the_columns_they_name() {
        assert_eq!(FTS_COLUMNS[SNIPPET_COLUMN], "transcript");
        assert_eq!(FTS_COLUMNS[SNIPPET_FALLBACK_COLUMN], "notes");
    }
}
