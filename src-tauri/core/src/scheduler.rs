//! Background scheduler: drains the processing queue when the machine is
//! idle, one recording at a time.
//!
//! The decision logic is the pure [`tick`] function so it can be tested
//! without threads or timers. [`run_loop`] is the thin thread wrapper the
//! app spawns; it parks between ticks and can be woken early (a user pressing
//! "Process now") via the returned [`Waker`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, Thread};
use std::time::Duration;

use anyhow::Result;

use crate::pipeline::llm::LlmClient;
use crate::pipeline::run::{process_recording, PipelineDeps};
use crate::models::ModelCache;
use crate::queue::{IdleSource, Queue, RunOutcome};

/// How long the loop sleeps between ticks when it has nothing to do. A wake
/// (see [`Waker`]) cuts a sleep short.
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// The settings a single scheduling decision reads. The scheduler obtains a
/// fresh copy before each tick so a saved template applies without requiring a
/// desktop-app restart.
pub struct SchedulerConfig {
    pub llm: LlmClient,
    pub task_models: BTreeMap<String, String>,
    pub templates: Vec<crate::templates::Template>,
    pub summary_prompt: String,
}

/// One scheduling decision: if the machine is idle, run at most one queued
/// recording through the pipeline. Models are acquired only after
/// `Queue::run_one` has passed both the idle gate and the queued-recording
/// check. The cache sweep is deliberately at the end of this function — the
/// scheduler tick is the only idle sweeper.
pub fn tick(
    queue: &Queue,
    idle: &dyn IdleSource,
    cache: &ModelCache,
    llm: &LlmClient,
    tasks: &[String],
    task_models: &BTreeMap<String, String>,
    templates: &[crate::templates::Template],
    summary_prompt: &str,
) -> Result<RunOutcome> {
    let outcome = queue.run_one(idle, |rec| {
        let task_llm = rec
            .task
            .as_deref()
            .and_then(|task| task_models.get(task))
            .map(|model| LlmClient {
                base_url: llm.base_url.clone(),
                model: model.clone(),
            });
        let lease = cache.acquire()?;
        let deps = PipelineDeps {
            transcriber: lease.transcriber(),
            diarizer: lease.diarizer(),
            llm: task_llm.as_ref().unwrap_or(llm),
            templates,
            summary_prompt,
            tasks: tasks.to_vec(),
        };
        let result = process_recording(queue.store, &deps, rec).map(|_| ());
        // Keep the lease across every pipeline stage, including the final
        // metadata write, then release it before the tick's idle sweep.
        drop(lease);
        result
    })?;
    cache.sweep();
    Ok(outcome)
}

/// Direct-dependency seam retained for small scheduler tests and callers that
/// already own a loaded model set. Production scheduling uses [`tick`] so
/// model lifetime is governed by [`ModelCache`].
pub fn tick_with_deps(
    queue: &Queue,
    idle: &dyn IdleSource,
    deps: &PipelineDeps,
) -> Result<RunOutcome> {
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

/// Runs the cache-backed scheduler loop on the current thread until stopped.
/// `on_outcome` is called after every tick (the app uses it to index finished
/// recordings); tests use it to observe progress. `tick_interval` is separate
/// from the model idle window and is injectable in tests.
pub fn run_loop<F, C>(
    queue: &Queue,
    idle: &dyn IdleSource,
    cache: &ModelCache,
    tasks: &[String],
    config: C,
    stop: Arc<AtomicBool>,
    on_outcome: F,
) where
    F: FnMut(&RunOutcome),
    C: Fn() -> SchedulerConfig,
{
    run_loop_with_interval(
        queue,
        idle,
        cache,
        tasks,
        config,
        stop,
        TICK_INTERVAL,
        on_outcome,
    );
}

/// Test-injectable version of [`run_loop`].
pub fn run_loop_with_interval<F, C>(
    queue: &Queue,
    idle: &dyn IdleSource,
    cache: &ModelCache,
    tasks: &[String],
    config: C,
    stop: Arc<AtomicBool>,
    tick_interval: Duration,
    mut on_outcome: F,
) where
    F: FnMut(&RunOutcome),
    C: Fn() -> SchedulerConfig,
{
    while !stop.load(Ordering::SeqCst) {
        let config = config();
        match tick(queue, idle, cache, &config.llm, tasks, &config.task_models, &config.templates, &config.summary_prompt) {
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
        thread::park_timeout(tick_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ModelIdleUnload, SpeechEngine};
    use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
    use crate::pipeline::llm::LlmClient;
    use crate::pipeline::transcribe::Transcriber;
    use crate::models::cache::{LoadedModels, ModelPaths};
    use crate::queue::AlwaysIdle;
    use crate::storage::{Mode, Status, Store};
    use chrono::TimeZone;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        templates: &'a [crate::templates::Template],
    ) -> PipelineDeps<'a> {
        PipelineDeps {
            transcriber,
            diarizer,
            llm,
            templates,
            summary_prompt: "",
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
        let templates = crate::templates::defaults();
        let deps = deps(&t, &d, &llm, &templates);

        let outcome = tick_with_deps(&queue, &AlwaysIdle, &deps).unwrap();
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
        let templates = crate::templates::defaults();
        let deps = deps(&t, &d, &llm, &templates);

        let outcome = tick_with_deps(&queue, &NeverIdle, &deps).unwrap();
        assert_eq!(outcome, RunOutcome::NotIdle);
        // Untouched: still queued, no attempts.
        let on_disk = store.scan().unwrap();
        assert_eq!(on_disk[0].meta.status, Status::Queued);
        assert_eq!(on_disk[0].meta.attempts, 0);
    }

    #[test]
    fn cache_backed_tick_does_not_load_models_until_idle_queued_work_exists() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let queue = Queue { store: &store };
        let loads = std::sync::Arc::new(AtomicUsize::new(0));
        let loader_loads = std::sync::Arc::clone(&loads);
        let cache = ModelCache::new(
            ModelPaths {
                speech: PathBuf::new(),
                segmentation: PathBuf::new(),
                embedding: PathBuf::new(),
                sense_voice: None,
                speech_engine: SpeechEngine::Whisper,
            },
            ModelIdleUnload::Never,
            move |_| {
                loader_loads.fetch_add(1, Ordering::SeqCst);
                Ok(LoadedModels {
                    transcriber: Box::new(EmptyText),
                    diarizer: Box::new(OneSpeaker),
                })
            },
            None,
        );
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "x".to_string(),
        };
        let tasks = Vec::new();
        let templates = crate::templates::defaults();

        let outcome = tick(&queue, &NeverIdle, &cache, &llm, &tasks, &BTreeMap::new(), &templates, "").unwrap();
        assert_eq!(outcome, RunOutcome::NotIdle);
        assert_eq!(loads.load(Ordering::SeqCst), 0);

        let mut rec = wav_recording(&store, "Lecture");
        queue.enqueue(&mut rec).unwrap();
        let outcome = tick(&queue, &AlwaysIdle, &cache, &llm, &tasks, &BTreeMap::new(), &templates, "").unwrap();
        assert!(matches!(
            outcome,
            RunOutcome::FailedWillRetry | RunOutcome::Ran
        ));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }
}
