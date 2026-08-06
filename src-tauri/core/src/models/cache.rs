//! Lazy, lease-backed ownership of the speech and speaker models.
//!
//! The cache is deliberately small and boring: one [`Mutex<Slot>`] owns the
//! optional loaded model set, and every processing job holds a [`ModelLease`]
//! until the pipeline returns. The scheduler is the only caller of
//! [`ModelCache::sweep`], so an idle timeout can never race an independent
//! watcher into dropping a model underneath a running job.
//!
//! The lease/unload shape is adapted from Handy's MIT-licensed transcription
//! manager (`cjpais/Handy`, `src-tauri/src/managers/transcription.rs`). Handy
//! uses a last-activity watcher; Notetaker keeps the same RAII safety property
//! while making the scheduler tick the single idle sweeper.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::api::{ModelIdleUnload, SpeechEngine};
use crate::pipeline::diarize::Diarizer;
use crate::pipeline::transcribe::Transcriber;

/// Paths needed to construct the models. Keeping these paths, rather than
/// constructed native model handles, is what lets startup stay cheap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPaths {
    pub speech: PathBuf,
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
    pub sense_voice: Option<(PathBuf, PathBuf)>,
    pub speech_engine: SpeechEngine,
}

/// The complete set of native handles used by one processing job.
pub struct LoadedModels {
    pub transcriber: Box<dyn Transcriber + Send + Sync>,
    pub diarizer: Box<dyn Diarizer + Send + Sync>,
}

/// Name retained for the runtime's synchronous test seam.
pub type SchedulerModels = LoadedModels;

/// A model loader is called outside the cache mutex. It must construct a
/// complete set or return an error; the cache never publishes a half-loaded
/// set to a lease.
pub type ModelLoader = dyn Fn(&ModelPaths) -> Result<LoadedModels> + Send + Sync + 'static;

/// Receives model lifecycle events. The Tauri shell turns these into the
/// `model-state-changed` event; core and the served UI can leave it unset.
pub type ModelEventSink = dyn Fn(ModelStateEvent) + Send + Sync + 'static;

/// State exposed to the UI when the cache changes state.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelState {
    Loading,
    Ready,
    Sleeping,
    Failed,
}

/// Payload for the Tauri `model-state-changed` event.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStateEvent {
    pub state: ModelState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ModelStateEvent {
    fn state(state: ModelState) -> Self {
        Self { state, error: None }
    }

    fn failed(error: String) -> Self {
        Self {
            state: ModelState::Failed,
            error: Some(error),
        }
    }
}

/// The one mutable slot protected by the cache mutex.
struct Slot {
    models: Option<Arc<LoadedModels>>,
    leases: usize,
    loading: bool,
    last_used: Option<Instant>,
}

struct CacheInner {
    /// This is intentionally a single slot. The models are one coherent set:
    /// Whisper/SenseVoice routing and diarization must come from the same
    /// load, never from a mixture of generations.
    slot: Mutex<Slot>,
    loading_done: Condvar,
    paths: ModelPaths,
    loader: Arc<dyn Fn(&ModelPaths) -> Result<Arc<LoadedModels>> + Send + Sync>,
    policy: Mutex<ModelIdleUnload>,
    event_sink: Mutex<Option<Arc<ModelEventSink>>>,
}

/// Lazily loads and safely unloads the speech/speaker model set.
#[derive(Clone)]
pub struct ModelCache {
    inner: Arc<CacheInner>,
}

/// A borrow-like handle that keeps the native models alive for one job.
pub struct ModelLease {
    inner: Arc<CacheInner>,
    models: Arc<LoadedModels>,
}

impl ModelCache {
    /// Builds a cache whose first [`acquire`](Self::acquire) calls `loader`.
    pub fn new<F>(
        paths: ModelPaths,
        policy: ModelIdleUnload,
        loader: F,
        event_sink: Option<Arc<ModelEventSink>>,
    ) -> Self
    where
        F: Fn(&ModelPaths) -> Result<LoadedModels> + Send + Sync + 'static,
    {
        let loader: Arc<dyn Fn(&ModelPaths) -> Result<Arc<LoadedModels>> + Send + Sync> =
            Arc::new(move |paths| loader(paths).map(Arc::new));
        Self::from_loader(paths, policy, loader, event_sink)
    }

    /// Creates a cache around already-constructed handles. This is useful for
    /// synchronous tests and for the compatibility `Runtime::start_scheduler`
    /// entry point; production startup uses [`Self::new`] so the handles are
    /// not constructed until a queued job actually needs them.
    pub fn from_loaded(
        models: LoadedModels,
        policy: ModelIdleUnload,
        event_sink: Option<Arc<ModelEventSink>>,
    ) -> Self {
        let models = Arc::new(models);
        let loader: Arc<dyn Fn(&ModelPaths) -> Result<Arc<LoadedModels>> + Send + Sync> =
            Arc::new(move |_| Ok(Arc::clone(&models)));
        Self::from_loader(
            ModelPaths {
                speech: PathBuf::new(),
                segmentation: PathBuf::new(),
                embedding: PathBuf::new(),
                sense_voice: None,
                speech_engine: SpeechEngine::Auto,
            },
            policy,
            loader,
            event_sink,
        )
    }

    fn from_loader(
        paths: ModelPaths,
        policy: ModelIdleUnload,
        loader: Arc<dyn Fn(&ModelPaths) -> Result<Arc<LoadedModels>> + Send + Sync>,
        event_sink: Option<Arc<ModelEventSink>>,
    ) -> Self {
        Self {
            inner: Arc::new(CacheInner {
                slot: Mutex::new(Slot {
                    models: None,
                    leases: 0,
                    loading: false,
                    last_used: None,
                }),
                loading_done: Condvar::new(),
                paths,
                loader,
                policy: Mutex::new(policy),
                event_sink: Mutex::new(event_sink),
            }),
        }
    }

    /// Replaces the idle policy without touching an active lease.
    pub fn set_policy(&self, policy: ModelIdleUnload) {
        *lock(&self.inner.policy) = policy;
    }

    /// The current policy, primarily for diagnostics and tests.
    pub fn policy(&self) -> ModelIdleUnload {
        *lock(&self.inner.policy)
    }

    /// Installs or replaces the callback used for model lifecycle events.
    pub fn set_event_sink(&self, event_sink: Arc<ModelEventSink>) {
        *lock(&self.inner.event_sink) = Some(event_sink);
    }

    /// Acquires the model set, loading it once if necessary.
    ///
    /// A second acquirer waits on the condition variable while the first
    /// loader is outside the mutex. It therefore observes the same loaded
    /// generation rather than starting a duplicate native load.
    pub fn acquire(&self) -> Result<ModelLease> {
        let models = loop {
            let mut slot = lock(&self.inner.slot);
            if let Some(models) = slot.models.as_ref().map(Arc::clone) {
                slot.leases += 1;
                break models;
            }
            if slot.loading {
                let _guard = self
                    .inner
                    .loading_done
                    .wait(slot)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            }

            slot.loading = true;
            drop(slot);
            break self.load()?;
        };

        Ok(ModelLease {
            inner: Arc::clone(&self.inner),
            models,
        })
    }

    /// The scheduler's one idle sweeper. Returns true only when it removed the
    /// cache's owning reference to a model set.
    pub fn sweep(&self) -> bool {
        self.sweep_at(Instant::now())
    }

    /// Clock-injectable form used by unit tests and acceptance harnesses.
    pub fn sweep_at(&self, now: Instant) -> bool {
        let policy = self.policy();
        let Some(window) = policy.idle_window() else {
            return false;
        };

        let did_unload = {
            let mut slot = lock(&self.inner.slot);
            let Some(last_used) = slot.last_used else {
                return false;
            };
            if slot.loading
                || slot.leases != 0
                || slot.models.is_none()
                || now.saturating_duration_since(last_used) < window
            {
                return false;
            }
            slot.last_used = None;
            let models = slot.models.take();
            // Keep the slot locked until the native destructor has run. A
            // racing acquirer may then reload, but it can never make the old
            // and new native generations overlap in memory.
            drop(models);
            true
        };

        if did_unload {
            // A live lease, if any, would have prevented this branch, so this
            // is the point at which native resources are free.
            self.emit(ModelStateEvent::state(ModelState::Sleeping));
            true
        } else {
            false
        }
    }

    /// Whether a loaded model set is currently retained by the cache.
    pub fn is_loaded(&self) -> bool {
        lock(&self.inner.slot).models.is_some()
    }

    /// Number of live leases. Exposed for diagnostics and race tests.
    pub fn leases(&self) -> usize {
        lock(&self.inner.slot).leases
    }

    /// Whether one thread is currently constructing the native model set.
    pub fn is_loading(&self) -> bool {
        lock(&self.inner.slot).loading
    }

    fn load(&self) -> Result<Arc<LoadedModels>> {
        self.emit(ModelStateEvent::state(ModelState::Loading));
        let paths = self.inner.paths.clone();
        let loaded = catch_unwind(AssertUnwindSafe(|| (self.inner.loader)(&paths)))
            .map_err(|_| anyhow!("model loader panicked"))
            .and_then(|result| result);

        match loaded {
            Ok(models) => {
                let mut slot = lock(&self.inner.slot);
                slot.models = Some(Arc::clone(&models));
                slot.leases += 1;
                slot.loading = false;
                slot.last_used = Some(Instant::now());
                self.inner.loading_done.notify_all();
                drop(slot);
                self.emit(ModelStateEvent::state(ModelState::Ready));
                Ok(models)
            }
            Err(error) => {
                let mut slot = lock(&self.inner.slot);
                slot.loading = false;
                self.inner.loading_done.notify_all();
                let message = error.to_string();
                drop(slot);
                self.emit(ModelStateEvent::failed(message));
                Err(error)
            }
        }
    }

    fn emit(&self, event: ModelStateEvent) {
        let sink = lock(&self.inner.event_sink).clone();
        if let Some(sink) = sink {
            sink(event);
        }
    }
}

impl ModelLease {
    pub fn transcriber(&self) -> &dyn Transcriber {
        &*self.models.transcriber
    }

    pub fn diarizer(&self) -> &dyn Diarizer {
        &*self.models.diarizer
    }
}

impl Drop for ModelLease {
    fn drop(&mut self) {
        let mut slot = lock(&self.inner.slot);
        debug_assert!(slot.leases > 0, "a model lease was dropped twice");
        if slot.leases > 0 {
            slot.leases -= 1;
            // The idle clock starts when the last job releases its lease, not
            // when a long-running job first acquired it.
            if slot.leases == 0 {
                slot.last_used = Some(Instant::now());
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ModelIdleUnload;
    use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
    use crate::pipeline::transcribe::Transcriber;
    use anyhow::Result;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Barrier};
    use std::thread;
    use std::time::Duration;

    struct EmptyTranscriber;
    impl Transcriber for EmptyTranscriber {
        fn transcribe(
            &self,
            _: &[f32],
            _: &[(f32, f32)],
        ) -> Result<Vec<(f32, f32, String)>> {
            Ok(Vec::new())
        }
    }

    struct EmptyDiarizer;
    impl Diarizer for EmptyDiarizer {
        fn diarize(&self, _: &[f32]) -> Result<Vec<SpeakerSpan>> {
            Ok(Vec::new())
        }
    }

    fn models() -> LoadedModels {
        LoadedModels {
            transcriber: Box::new(EmptyTranscriber),
            diarizer: Box::new(EmptyDiarizer),
        }
    }

    fn cache(
        policy: ModelIdleUnload,
        loads: Arc<AtomicUsize>,
        loader_barrier: Option<(Arc<Barrier>, Arc<Barrier>)>,
    ) -> ModelCache {
        ModelCache::new(
            ModelPaths {
                speech: PathBuf::from("speech"),
                segmentation: PathBuf::from("segmentation"),
                embedding: PathBuf::from("embedding"),
                sense_voice: None,
                speech_engine: SpeechEngine::Auto,
            },
            policy,
            move |_| {
                loads.fetch_add(1, Ordering::SeqCst);
                if let Some((started, release)) = &loader_barrier {
                    started.wait();
                    release.wait();
                }
                Ok(models())
            },
            None,
        )
    }

    #[test]
    fn load_is_on_demand_and_only_happens_once_for_a_live_generation() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = cache(ModelIdleUnload::Never, Arc::clone(&loads), None);

        assert!(!cache.is_loaded());
        let lease = cache.acquire().unwrap();
        assert!(cache.is_loaded());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        drop(lease);
        let second = cache.acquire().unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        drop(second);
    }

    #[test]
    fn two_acquirers_during_a_load_share_one_result() {
        let loads = Arc::new(AtomicUsize::new(0));
        let loader_started = Arc::new(Barrier::new(2));
        let release_loader = Arc::new(Barrier::new(2));
        let cache = Arc::new(cache(
            ModelIdleUnload::Never,
            Arc::clone(&loads),
            Some((Arc::clone(&loader_started), Arc::clone(&release_loader))),
        ));

        let first_cache = Arc::clone(&cache);
        let first = thread::spawn(move || first_cache.acquire().unwrap());
        loader_started.wait();

        let second_cache = Arc::clone(&cache);
        let second = thread::spawn(move || second_cache.acquire().unwrap());
        // The loader is still held here. The second caller must be waiting,
        // not starting a second native construction.
        assert!(cache.is_loading());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        release_loader.wait();
        let first_lease = first.join().unwrap();
        let second_lease = second.join().unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(cache.leases(), 2);
        drop(first_lease);
        drop(second_lease);
    }

    #[test]
    fn unload_racing_acquire_never_removes_a_live_lease() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(cache(ModelIdleUnload::AfterBatch, Arc::clone(&loads), None));
        drop(cache.acquire().unwrap());

        let start = Arc::new(Barrier::new(3));
        let acquirer_cache = Arc::clone(&cache);
        let acquirer_start = Arc::clone(&start);
        let acquirer = thread::spawn(move || {
            acquirer_start.wait();
            let lease = acquirer_cache.acquire().unwrap();
            assert!(lease.transcriber().transcribe(&[], &[]).is_ok());
            lease
        });

        let sweeper_cache = Arc::clone(&cache);
        let sweeper_start = Arc::clone(&start);
        let sweeper = thread::spawn(move || {
            sweeper_start.wait();
            sweeper_cache.sweep_at(Instant::now() + Duration::from_secs(1));
        });

        start.wait();
        sweeper.join().unwrap();
        let lease = acquirer.join().unwrap();
        assert_eq!(cache.leases(), 1);
        assert!(cache.is_loaded());
        drop(lease);
        // The sweeper may legitimately win before the acquirer and force one
        // reload. What must never happen is removing the set after the
        // acquirer has obtained its lease.
        assert!(
            (1..=2).contains(&loads.load(Ordering::SeqCst)),
            "the race may load at most one replacement set"
        );
        assert!(cache.sweep_at(Instant::now() + Duration::from_secs(1)));
    }

    #[test]
    fn a_lease_held_past_the_idle_window_prevents_unload() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = cache(ModelIdleUnload::FiveMinutes, Arc::clone(&loads), None);
        let lease = cache.acquire().unwrap();
        let old = Instant::now() + Duration::from_secs(3600);

        assert!(!cache.sweep_at(old));
        assert!(lease.transcriber().transcribe(&[], &[]).is_ok());
        drop(lease);
        assert!(cache.sweep_at(Instant::now() + Duration::from_secs(3600)));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn after_batch_unloads_on_the_next_scheduler_sweep() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = cache(ModelIdleUnload::AfterBatch, Arc::clone(&loads), None);
        drop(cache.acquire().unwrap());

        assert!(cache.sweep_at(Instant::now()));
        assert!(!cache.is_loaded());
    }

    #[test]
    fn never_keeps_the_loaded_set_even_after_a_long_idle_window() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = cache(ModelIdleUnload::Never, Arc::clone(&loads), None);
        drop(cache.acquire().unwrap());

        assert!(!cache.sweep_at(Instant::now() + Duration::from_secs(86_400)));
        assert!(cache.is_loaded());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn load_and_unload_emit_ready_and_sleeping_states() {
        let (tx, rx) = mpsc::channel();
        let sink: Arc<ModelEventSink> = Arc::new(move |event| tx.send(event).unwrap());
        let loads = Arc::new(AtomicUsize::new(0));
        let cache = ModelCache::new(
            ModelPaths {
                speech: PathBuf::new(),
                segmentation: PathBuf::new(),
                embedding: PathBuf::new(),
                sense_voice: None,
                speech_engine: SpeechEngine::Auto,
            },
            ModelIdleUnload::AfterBatch,
            move |_| {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(models())
            },
            Some(sink),
        );

        drop(cache.acquire().unwrap());
        assert_eq!(rx.recv().unwrap().state, ModelState::Loading);
        assert_eq!(rx.recv().unwrap().state, ModelState::Ready);
        assert!(cache.sweep_at(Instant::now()));
        assert_eq!(rx.recv().unwrap().state, ModelState::Sleeping);
    }
}
