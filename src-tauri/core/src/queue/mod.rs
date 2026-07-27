//! The processing queue: crash-safe status transitions for recordings
//! waiting to be diarized/transcribed/summarized.
//!
//! There is no separate queue state file. The queue is entirely derived
//! from `meta.json` on disk (via `Store::scan`), which is what makes it
//! crash-safe: a process killed mid-processing leaves an accurate
//! `Processing` status on disk, and a later startup sweep can requeue it.

use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::storage::{RecordingRef, Status, Store};

/// Whether the machine is idle enough to start a processing job right now.
/// The real macOS idle-time implementation lives elsewhere (Plan B); this
/// crate only needs the trait plus a trivial always-true impl for
/// Linux/tests.
pub trait IdleSource: Send + Sync {
    fn ok_to_run(&self) -> bool;
}

/// Reports idle unconditionally. Used on Linux (no idle-time API) and in
/// tests.
pub struct AlwaysIdle;

impl IdleSource for AlwaysIdle {
    fn ok_to_run(&self) -> bool {
        true
    }
}

/// Result of a single `Queue::run_one` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The closure ran and succeeded; the recording is now `Ready`.
    Ran,
    /// Nothing was `Queued`; `run_one` did nothing.
    NothingQueued,
    /// `idle.ok_to_run()` was false; `run_one` did nothing, closure never ran.
    NotIdle,
    /// The closure failed but `attempts` is still under the retry limit;
    /// the recording went back to `Queued`.
    FailedWillRetry,
    /// The closure failed and `attempts` hit the retry limit; the
    /// recording is now `Failed` with `meta.error` set.
    FailedFinal,
}

/// Retry ceiling: the third failed attempt is the last one.
const MAX_ATTEMPTS: u32 = 3;

pub struct Queue<'a> {
    pub store: &'a Store,
}

impl<'a> Queue<'a> {
    /// `Recorded`/`Failed` -> `Queued`, persisted immediately. Any other
    /// status is a no-op (not an error): enqueue is meant to be safe to
    /// call speculatively — e.g. a startup sweep re-enqueueing everything
    /// it finds eligible — without clobbering a recording that is already
    /// `Queued`, `Processing`, or `Ready`.
    pub fn enqueue(&self, rec: &mut RecordingRef) -> Result<()> {
        match rec.meta.status {
            Status::Recorded | Status::Failed => {
                rec.meta.status = Status::Queued;
                // Clear any error from a previous failed attempt so the UI
                // does not show a stale "download interrupted" on a
                // recording that is now queued to run again.
                //
                // `meta.capture_note` is deliberately left alone: it explains
                // the audio, not the attempt. Queueing a recording cannot undo
                // the disk filling up mid-lecture, so wiping that message here
                // would leave the user with a 20-minute file of a 40-minute
                // class and no explanation.
                rec.meta.error = None;
                self.store.save_meta(rec)?;
            }
            Status::Queued | Status::Processing | Status::Ready => {}
        }
        Ok(())
    }

    /// The oldest `Queued` recording by `Meta.created`, freshly derived
    /// from disk on every call — there is no in-memory queue to go stale.
    pub fn next(&self) -> Result<Option<RecordingRef>> {
        let mut queued: Vec<RecordingRef> = self
            .store
            .scan()?
            .into_iter()
            .filter(|r| r.meta.status == Status::Queued)
            .collect();
        queued.sort_by_key(created_key);
        Ok(queued.into_iter().next())
    }

    /// Runs at most one queued recording through `process`.
    ///
    /// Status is written to disk via `store.save_meta` both before and
    /// after `process` runs: `Processing` is persisted first, so a crash
    /// mid-processing leaves that status on disk for a later startup
    /// sweep to find and requeue, rather than leaving the recording
    /// silently stuck as `Queued` forever or lost.
    pub fn run_one<F>(&self, idle: &dyn IdleSource, process: F) -> Result<RunOutcome>
    where
        F: FnOnce(&RecordingRef) -> Result<()>,
    {
        if !idle.ok_to_run() {
            return Ok(RunOutcome::NotIdle);
        }

        let mut rec = match self.next()? {
            Some(r) => r,
            None => return Ok(RunOutcome::NothingQueued),
        };

        rec.meta.status = Status::Processing;
        self.store.save_meta(&rec)?;

        let result = process(&rec);

        // `process` may enrich meta.json on disk (the pipeline writes
        // `speakers` and `stages`). Reload before flipping status so this
        // status-only write builds on the enriched copy instead of clobbering
        // it with the stale pre-processing snapshot. Reload can fail if the
        // recording folder vanished mid-run; fall back to the in-memory copy.
        let mut rec = self.store.reload(&rec).unwrap_or(rec);

        match result {
            Ok(()) => {
                rec.meta.status = Status::Ready;
                self.store.save_meta(&rec)?;
                Ok(RunOutcome::Ran)
            }
            Err(e) => {
                rec.meta.attempts += 1;
                if rec.meta.attempts >= MAX_ATTEMPTS {
                    rec.meta.status = Status::Failed;
                    rec.meta.error = Some(e.to_string());
                    self.store.save_meta(&rec)?;
                    Ok(RunOutcome::FailedFinal)
                } else {
                    rec.meta.status = Status::Queued;
                    self.store.save_meta(&rec)?;
                    Ok(RunOutcome::FailedWillRetry)
                }
            }
        }
    }
}

/// Sort key for `Meta.created` (RFC3339), used to find the oldest queued
/// recording. An unparseable timestamp sorts to the front rather than
/// panicking or aborting `next()` — malformed data shouldn't wedge the
/// queue.
fn created_key(rec: &RecordingRef) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(&rec.meta.created).unwrap_or_else(|_| {
        DateTime::parse_from_rfc3339("1970-01-01T00:00:00Z").expect("valid constant")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Mode, StageTiming};
    use anyhow::anyhow;
    use chrono::TimeZone;

    #[test]
    fn successful_run_preserves_pipeline_enriched_meta() {
        // Regression for the bug where run_one re-saved its stale pre-run
        // snapshot after the pipeline had enriched meta.json, wiping speakers
        // and stage timings on every successful run.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };
        let mut rec = recorded(&store, "Lecture", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap();

        // The closure stands in for process_recording: it enriches meta.json
        // on disk (speakers + stages) without touching status.
        let outcome = queue
            .run_one(&AlwaysIdle, |r| {
                let mut enriched = store.reload(r).unwrap();
                enriched
                    .meta
                    .speakers
                    .insert("spk1".to_string(), "Speaker 1".to_string());
                enriched.meta.stages.push(StageTiming {
                    stage: "diarize".to_string(),
                    ms: 12,
                });
                store.save_meta(&enriched).unwrap();
                Ok(())
            })
            .unwrap();

        assert_eq!(outcome, RunOutcome::Ran);
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Ready);
        assert_eq!(
            on_disk[0].meta.speakers.get("spk1").map(String::as_str),
            Some("Speaker 1"),
            "speakers written by the pipeline must survive the status flip"
        );
        assert_eq!(
            on_disk[0].meta.stages.len(),
            1,
            "stage timings must survive the status flip"
        );
    }

    /// Creates a fresh `Recorded` recording with an explicit timestamp, so
    /// tests can control ordering.
    fn recorded(
        store: &Store,
        title: &str,
        y: i32,
        m: u32,
        d: u32,
        h: u32,
        min: u32,
    ) -> RecordingRef {
        let created = chrono::Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap();
        store
            .create_recording(title, Mode::Meeting, created)
            .unwrap()
    }

    struct NeverIdle;
    impl IdleSource for NeverIdle {
        fn ok_to_run(&self) -> bool {
            false
        }
    }

    #[test]
    fn happy_path_recorded_to_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        assert_eq!(rec.meta.status, Status::Recorded);

        queue.enqueue(&mut rec).unwrap();
        assert_eq!(rec.meta.status, Status::Queued);
        assert_eq!(store.scan().unwrap()[0].meta.status, Status::Queued);

        let outcome = queue.run_one(&AlwaysIdle, |_r| Ok(())).unwrap();
        assert_eq!(outcome, RunOutcome::Ran);
        assert_eq!(store.scan().unwrap()[0].meta.status, Status::Ready);
    }

    #[test]
    fn enqueue_from_other_statuses_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap(); // Recorded -> Queued
        queue.enqueue(&mut rec).unwrap(); // already Queued -> no-op
        assert_eq!(rec.meta.status, Status::Queued);

        // Force to Ready directly (as run_one would) and confirm enqueue
        // refuses to touch it.
        rec.meta.status = Status::Ready;
        store.save_meta(&rec).unwrap();
        queue.enqueue(&mut rec).unwrap();
        assert_eq!(rec.meta.status, Status::Ready);
        assert_eq!(store.scan().unwrap()[0].meta.status, Status::Ready);
    }

    #[test]
    fn failure_retries_twice_then_fails_final_with_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap();

        let outcome = queue
            .run_one(&AlwaysIdle, |_r| Err(anyhow!("boom")))
            .unwrap();
        assert_eq!(outcome, RunOutcome::FailedWillRetry);
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.attempts, 1);

        let outcome = queue
            .run_one(&AlwaysIdle, |_r| Err(anyhow!("boom")))
            .unwrap();
        assert_eq!(outcome, RunOutcome::FailedWillRetry);
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.attempts, 2);

        let outcome = queue
            .run_one(&AlwaysIdle, |_r| Err(anyhow!("boom")))
            .unwrap();
        assert_eq!(outcome, RunOutcome::FailedFinal);
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Failed);
        assert_eq!(on_disk[0].meta.attempts, 3);
        assert!(on_disk[0].meta.error.as_deref().unwrap().contains("boom"));
    }

    #[test]
    fn re_enqueueing_a_failed_recording_clears_its_stale_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        // Drive a recording all the way to Failed with an error set.
        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap();
        for _ in 0..3 {
            queue.run_one(&AlwaysIdle, |_r| Err(anyhow!("boom"))).unwrap();
        }
        let mut failed = store.scan().unwrap().into_iter().next().unwrap();
        assert_eq!(failed.meta.status, Status::Failed);
        assert!(failed.meta.error.is_some());

        // Re-queuing it must not carry the old error forward.
        queue.enqueue(&mut failed).unwrap();
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.error, None);
    }

    #[test]
    fn a_capture_time_note_survives_enqueue_and_a_successful_run() {
        // The bug this pins: `Session::stop` explains a short recording ("the
        // disk was almost full"), then enqueue wiped that message and the user
        // never learned why their lecture was 20 minutes short.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Lecture", 2026, 8, 4, 9, 0);
        rec.meta.capture_note = Some("Recording stopped because the disk was almost full.".to_string());
        store.save_meta(&rec).unwrap();

        queue.enqueue(&mut rec).unwrap();
        assert_eq!(
            store.scan().unwrap()[0].meta.capture_note.as_deref(),
            Some("Recording stopped because the disk was almost full."),
            "queueing a recording must not erase why capture ended early"
        );

        assert_eq!(
            queue.run_one(&AlwaysIdle, |_r| Ok(())).unwrap(),
            RunOutcome::Ran
        );
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Ready);
        assert_eq!(
            on_disk[0].meta.capture_note.as_deref(),
            Some("Recording stopped because the disk was almost full."),
            "processing successfully does not undo what happened during capture"
        );
    }

    #[test]
    fn clearing_a_processing_error_on_re_enqueue_leaves_the_capture_note_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Lecture", 2026, 8, 4, 9, 0);
        rec.meta.capture_note = Some("The microphone stopped working partway through.".to_string());
        store.save_meta(&rec).unwrap();

        queue.enqueue(&mut rec).unwrap();
        for _ in 0..3 {
            queue.run_one(&AlwaysIdle, |_r| Err(anyhow!("boom"))).unwrap();
        }
        let mut failed = store.scan().unwrap().into_iter().next().unwrap();
        assert_eq!(failed.meta.status, Status::Failed);
        assert!(failed.meta.error.is_some());

        queue.enqueue(&mut failed).unwrap();
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.error, None, "the retryable error still clears");
        assert_eq!(
            on_disk[0].meta.capture_note.as_deref(),
            Some("The microphone stopped working partway through."),
            "a capture-time problem is not undone by retrying the processing"
        );
    }

    #[test]
    fn not_idle_leaves_status_queued_and_skips_closure() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap();

        let outcome = queue
            .run_one(&NeverIdle, |_r| {
                panic!("closure must not run when not idle")
            })
            .unwrap();
        assert_eq!(outcome, RunOutcome::NotIdle);

        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.attempts, 0);
    }

    #[test]
    fn next_returns_oldest_queued_by_created() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut later = recorded(&store, "Later", 2026, 8, 4, 12, 0);
        let mut earlier = recorded(&store, "Earlier", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut later).unwrap();
        queue.enqueue(&mut earlier).unwrap();

        let next = queue.next().unwrap().unwrap();
        assert_eq!(next.meta.title, "Earlier");
    }

    #[test]
    fn nothing_queued_is_reported_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let outcome = queue
            .run_one(&AlwaysIdle, |_r| panic!("closure must not run"))
            .unwrap();
        assert_eq!(outcome, RunOutcome::NothingQueued);
    }

    /// Crash-safety proof: while the closure is executing, `Processing`
    /// must already be on disk (not just held in memory), because that is
    /// exactly the state a killed process would leave behind for a
    /// startup sweep to find.
    #[test]
    fn processing_status_is_on_disk_while_closure_runs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };

        let mut rec = recorded(&store, "Standup", 2026, 8, 4, 9, 0);
        queue.enqueue(&mut rec).unwrap();

        let outcome = queue
            .run_one(&AlwaysIdle, |r| {
                // Re-scan through an independent Store handle, the way a
                // startup sweep after a crash would.
                let fresh = Store::new(&store.root);
                let rescanned = fresh.scan().unwrap();
                let found = rescanned
                    .iter()
                    .find(|x| x.meta.id == r.meta.id)
                    .expect("recording present during processing");
                assert_eq!(found.meta.status, Status::Processing);
                Ok(())
            })
            .unwrap();

        assert_eq!(outcome, RunOutcome::Ran);
    }
}
