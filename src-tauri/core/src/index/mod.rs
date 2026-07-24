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

impl Index {
    pub fn open(db_path: &Path) -> Result<Index> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recordings(
                id TEXT PRIMARY KEY, title TEXT, task TEXT, created TEXT,
                duration_s REAL, mode TEXT, status TEXT, dir TEXT
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS recordings_fts
                USING fts5(id UNINDEXED, title, transcript, summary);",
        )?;
        Ok(Index { conn })
    }

    /// Full rescan of `store`: clears both tables and re-indexes every
    /// recording found on disk, reading `transcript.md`/`summary.md` when
    /// present. Returns the number of recordings indexed.
    pub fn rebuild(&mut self, store: &Store) -> Result<usize> {
        let recs = store.scan()?;

        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM recordings", [])?;
        tx.execute("DELETE FROM recordings_fts", [])?;
        tx.commit()?;

        for rec in &recs {
            let transcript = std::fs::read_to_string(rec.dir.join("transcript.md")).unwrap_or_default();
            let summary = std::fs::read_to_string(rec.dir.join("summary.md")).unwrap_or_default();
            self.upsert(rec, &transcript, &summary)?;
        }
        Ok(recs.len())
    }

    /// Indexes (or re-indexes) a single recording. `transcript`/`summary`
    /// may be empty strings for a recording that hasn't been processed yet.
    pub fn upsert(&mut self, rec: &RecordingRef, transcript: &str, summary: &str) -> Result<()> {
        let id = &rec.meta.id;

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
            "INSERT INTO recordings_fts(id, title, transcript, summary) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                segment_cjk(&rec.meta.title),
                segment_cjk(transcript),
                segment_cjk(summary),
            ],
        )?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let match_expr = fts_phrase(query);
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.title, r.task,
                    snippet(recordings_fts, 2, '<b>', '</b>', '…', 12) AS snip
             FROM recordings_fts
             JOIN recordings r ON r.id = recordings_fts.id
             WHERE recordings_fts MATCH ?1
             ORDER BY rank",
        )?;
        let rows = stmt.query_map(params![match_expr], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let task: Option<String> = row.get(2)?;
            let snip: String = row.get(3)?;
            Ok(SearchHit {
                id,
                title,
                task,
                snippet: desegment_cjk(&snip),
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Budget sync", Mode::Meeting, created).unwrap();
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture", Mode::InPerson, created).unwrap();
        // Contains 预 and 算 far apart, but never the word 预算.
        std::fs::write(r.dir.join("transcript.md"), "Speaker 1: 预计的方案还要再算一次").unwrap();
        let mut ix = Index::open(&dir.path().join("ix.sqlite")).unwrap();
        ix.rebuild(&s).unwrap();

        assert_eq!(ix.search("预算").unwrap().len(), 0);
    }

    #[test]
    fn rebuild_indexes_transcripts_and_survives_db_delete() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Budget sync", Mode::Meeting, created).unwrap();
        std::fs::write(r.dir.join("transcript.md"), "George: the quarterly budget is late").unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();
        assert_eq!(ix.rebuild(&s).unwrap(), 1);
        assert_eq!(ix.search("quarterly budget").unwrap()[0].title, "Budget sync");
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        s.create_recording("Untouched lecture", Mode::InPerson, created).unwrap();
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Planning sync", Mode::Meeting, created).unwrap();
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
        assert_eq!(cn_hits2.len(), 1, "four-character Chinese phrase should match");

        let en_hits = ix.search("quarterly").unwrap();
        assert_eq!(en_hits.len(), 1, "English term should still match in the same bilingual corpus");
    }

    #[test]
    fn snippet_strips_inserted_cjk_spacing() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("root"));
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Planning sync", Mode::Meeting, created).unwrap();
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Budget sync", Mode::Meeting, created).unwrap();
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
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Budget sync", Mode::Meeting, created).unwrap();
        let db = dir.path().join("ix.sqlite");
        let mut ix = Index::open(&db).unwrap();

        ix.upsert(&r, "first draft transcript", "").unwrap();
        ix.upsert(&r, "revised transcript about pricing", "").unwrap();

        assert_eq!(ix.search("pricing").unwrap().len(), 1);
        assert_eq!(ix.search("first draft").unwrap().len(), 0);
    }
}
