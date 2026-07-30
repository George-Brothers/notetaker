//! Background scheduler: drains the processing queue when the machine is
//! idle, one recording at a time.
//!
//! The decision logic is the pure [`tick`] function so it can be tested
//! without threads or timers. [`run_loop`] is the thin thread wrapper the
//! app spawns; it parks between ticks and can be woken early (a user pressing
//! "Process now") via the returned [`Waker`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, Thread};
use std::time::Duration;

use anyhow::Result;

use crate::pipeline::run::{process_recording, PipelineDeps};
use crate::queue::{IdleSource, Queue, RunOutcome};

/// How long the loop sleeps between ticks when it has nothing to do. A wake
/// (see [`Waker`]) cuts a sleep short.
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// One scheduling decision: if the machine is idle, run at most one queued
/// recording through the pipeline. Returns what happened so a caller (or a
/// test) can react. Pure with respect to time — no sleeping, no threads.
pub fn tick(queue: &Queue, idle: &dyn IdleSource, deps: &PipelineDeps) -> Result<RunOutcome> {
    queue.run_one(idle, |rec| {
        process_recording(queue.store, deps, rec).map(|_| ())
    })
}

/// Wakes a parked scheduler loop early. Cloneable and `Send` so a "Process
/// now" command from any thread can cut the current sleep short.
#[derive(Clone)]
pub struct Waker {
    thread: Thread,
    stop: Arc<AtomicBool>,
}

impl Waker {
    /// Interrupts the loop's current sleep so it ticks now.
    pub fn wake(&self) {
        self.thread.unpark();
    }

    /// Asks the loop to exit after its current tick.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// Runs the scheduler loop on the current thread until stopped. Intended to
/// be given its own `std::thread`. `on_outcome` is called after every tick
/// (the app uses it to emit `queue-changed` / `processing-progress` events);
/// tests use it to observe progress.
///
/// The caller owns the thread and the [`Waker`]: spawn a thread running this,
/// then build a `Waker` from that thread's handle and the same `stop` flag so
/// a "Process now" command can cut the current sleep short. Wiring the actual
/// `std::thread::spawn` lives in the app layer (Plan B), because it needs
/// `'static` model handles the core can't own.
pub fn run_loop<F>(
    queue: &Queue,
    idle: &dyn IdleSource,
    deps: &PipelineDeps,
    stop: Arc<AtomicBool>,
    mut on_outcome: F,
) where
    F: FnMut(&RunOutcome),
{
    while !stop.load(Ordering::SeqCst) {
        match tick(queue, idle, deps) {
            Ok(outcome) => {
                let more_now = matches!(outcome, RunOutcome::Ran);
                on_outcome(&outcome);
                // If we just processed one and there may be more queued, loop
                // again immediately rather than sleeping a full interval.
                if more_now {
                    continue;
                }
            }
            Err(_) => {
                // A tick error is already recorded on the recording's meta by
                // the queue (status Failed / error text); don't kill the loop
                // over one bad recording.
            }
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        thread::park_timeout(TICK_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
    use crate::pipeline::llm::LlmClient;
    use crate::pipeline::transcribe::Transcriber;
    use crate::queue::AlwaysIdle;
    use crate::storage::{Mode, Status, Store};
    use chrono::TimeZone;

    struct NeverIdle;
    impl IdleSource for NeverIdle {
        fn ok_to_run(&self) -> bool {
            false
        }
    }

    // Trivial stage stubs so `tick` can run without real models. The
    // in-person path calls diarize then transcribe then the LLM; we point the
    // LLM at a dead port and only assert on whether the closure RAN, so we
    // make the pipeline fail fast at the LLM step. That still exercises the
    // scheduler's idle-gating decision, which is what `tick` owns.
    struct OneSpeaker;
    impl Diarizer for OneSpeaker {
        fn diarize(&self, _: &[f32]) -> Result<Vec<SpeakerSpan>> {
            Ok(vec![SpeakerSpan {
                start_s: 0.0,
                end_s: 1.0,
                speaker: 0,
            }])
        }
    }
    struct EmptyText;
    impl Transcriber for EmptyText {
        fn transcribe(&self, _: &[f32], _: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>> {
            Ok(vec![(0.0, 1.0, "hello".to_string())])
        }
    }

    fn wav_recording(store: &Store, title: &str) -> crate::storage::RecordingRef {
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let rec = store
            .create_recording(title, Mode::InPerson, created)
            .unwrap();
        // Minimal valid 16 kHz mono WAV (0.1 s of silence) so load_mono_16k
        // succeeds and the run reaches the LLM step.
        write_silence_wav(&rec.dir.join("audio-mic.wav"));
        rec
    }

    fn write_silence_wav(path: &std::path::Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..1600 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn deps<'a>(
        transcriber: &'a EmptyText,
        diarizer: &'a OneSpeaker,
        llm: &'a LlmClient,
    ) -> PipelineDeps<'a> {
        PipelineDeps {
            transcriber,
            diarizer,
            llm,
            tasks: vec![],
        }
    }

    #[test]
    fn tick_runs_a_queued_recording_when_idle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };
        let mut rec = wav_recording(&store, "Lecture");
        queue.enqueue(&mut rec).unwrap();

        // LLM points at a dead port: process_recording will fail at
        // summarize, so run_one reports the run happened but failed. Either
        // way the scheduler DID pick it up — status leaves Queued.
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "x".to_string(),
        };
        let (t, d) = (EmptyText, OneSpeaker);
        let deps = deps(&t, &d, &llm);

        let outcome = tick(&queue, &AlwaysIdle, &deps).unwrap();
        // The recording was picked up and attempted (LLM failure → retry).
        assert!(matches!(
            outcome,
            RunOutcome::FailedWillRetry | RunOutcome::Ran
        ));
        // It is no longer sitting untouched: attempts incremented.
        let on_disk = store.scan().unwrap();
        assert!(on_disk[0].meta.attempts >= 1 || on_disk[0].meta.status == Status::Ready);
    }

    #[test]
    fn tick_does_nothing_when_not_idle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };
        let mut rec = wav_recording(&store, "Lecture");
        queue.enqueue(&mut rec).unwrap();

        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "x".to_string(),
        };
        let (t, d) = (EmptyText, OneSpeaker);
        let deps = deps(&t, &d, &llm);

        let outcome = tick(&queue, &NeverIdle, &deps).unwrap();
        assert_eq!(outcome, RunOutcome::NotIdle);
        // Untouched: still queued, no attempts.
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.attempts, 0);
    }
}
