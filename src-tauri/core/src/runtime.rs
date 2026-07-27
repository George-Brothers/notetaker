//! The single object the Tauri app crate holds, with one method per command.
//!
//! **Why this file exists at all.** The app crate needs webkit/gtk to compile,
//! which this build box does not have, so nothing in it can be tested until the
//! Mac day. Every line of logic that would normally live in a
//! `#[tauri::command]` therefore lives here instead, where it is exercised on
//! Linux against fakes. The wrappers the Mac day writes should each be one
//! line:
//!
//! ```ignore
//! #[tauri::command(rename_all = "camelCase")]
//! fn get_recording(state: State<Runtime>, id: String) -> Result<RecordingDetail, String> {
//!     state.get_recording(&id).map_err(|e| format!("{e:#}"))
//! }
//! ```
//!
//! `rename_all = "camelCase"` is stated on purpose. `src/lib/ipc.ts` sends
//! `summaryMd`, `appId`; the Rust parameters are `summary_md`, `app_id`. A
//! mismatch is not a compile error and not a visible crash — the invoke simply
//! rejects at runtime with a deserialization error the user reads as "nothing
//! happened". [`COMMANDS`] below is the written-down contract, and
//! `ipc_contract_matches_the_documented_command_table` fails the build if
//! either side drifts from it.
//!
//! **Interior mutability, not `&mut self`.** Tauri hands a command
//! `&State<T>`, so every method here takes `&self` and locks what it needs. The
//! `Runtime` is `Send + Sync` and cheap to clone (an `Arc` inside), so the
//! capture pump and the scheduler can hold the same state from their own
//! threads.
//!
//! **Platform seams are injected, never constructed here.** Audio comes from a
//! [`CaptureSources`] factory and idle/power readings from a
//! [`SystemProbe`]; B2 swaps the implementations and changes nothing else in
//! this file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::api::{self, RecordingDetail, RecordingRow, SearchHit, Settings};
use crate::capture::session::Session;
use crate::capture::source::{AudioSource, FakeSource};
use crate::capture::{CaptureState, CaptureStatus, DiskSpace};
use crate::index::Index;
use crate::models::{detect_tier, Tier};
use crate::ollama::{self, OllamaStatus, PullProgress};
use crate::pipeline::diarize::Diarizer;
use crate::pipeline::llm::LlmClient;
use crate::pipeline::run::{requeue_stale, PipelineDeps};
use crate::pipeline::transcribe::Transcriber;
use crate::power::{PowerPolicy, PowerState, SystemProbe};
use crate::queue::{IdleSource, Queue, RunOutcome};
use crate::scheduler;
use crate::storage::{Mode, RecordingRef, Status, Store};
use crate::watch::watcher::Watcher;
use crate::watch::{AutoRecordPolicy, MeetingEvent};

/// Written next to the audio by the pipeline. Duplicated from `api.rs`, which
/// keeps its own copies private.
const TRANSCRIPT_FILE: &str = "transcript.md";
const SUMMARY_FILE: &str = "summary.md";

/// File names inside the app's own data directory.
const SETTINGS_FILE: &str = "settings.json";
const INDEX_FILE: &str = "index.sqlite";

/// How often the capture thread moves audio from the sources into their files.
///
/// Sources buffer between reads, so this is a latency/wakeups trade-off rather
/// than a correctness one: ten pumps a second keeps the record bar's level
/// meters lively without waking the CPU constantly through an hour-long
/// lecture.
const PUMP_INTERVAL: Duration = Duration::from_millis(100);

/// How long a free-space reading is reused before the volume is measured
/// again.
///
/// The disk guard runs on every pump — ten times a second — and enumerating
/// mounted volumes is a syscall per mount. Capture writes about 32 KB/s per
/// track, so five seconds of staleness is well under a megabyte against a
/// `MIN_FREE_MB` floor of 500: the guard still trips with hundreds of megabytes
/// of headroom.
const DISK_POLL_INTERVAL: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The contract with the UI
// ---------------------------------------------------------------------------

/// One command the UI may invoke, with the exact argument names it sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The `#[tauri::command]` name, as written in `src/lib/ipc.ts`.
    pub name: &'static str,
    /// Argument names **in camelCase**, exactly as the UI's `invoke` object
    /// spells them. The app crate's wrapper must therefore carry
    /// `rename_all = "camelCase"`.
    pub args: &'static [&'static str],
}

/// Every command the UI can invoke. **This table is the contract**; it is
/// checked against `src/lib/ipc.ts` on every test run, and a command or an
/// argument added on one side without the other fails the suite.
///
/// The order matches `ipc.ts` so a human diff of the two reads straight down.
pub const COMMANDS: &[Command] = &[
    Command { name: "list_tasks", args: &[] },
    Command { name: "create_task", args: &["name"] },
    Command { name: "list_recordings", args: &[] },
    Command { name: "get_recording", args: &["id"] },
    Command { name: "search", args: &["query"] },
    Command { name: "process_now", args: &["id"] },
    Command { name: "update_summary", args: &["id", "summaryMd"] },
    Command { name: "assign_task", args: &["id", "task"] },
    Command { name: "rename_recording", args: &["id", "title"] },
    Command { name: "rename_speaker", args: &["id", "key", "name"] },
    Command { name: "get_settings", args: &[] },
    Command { name: "set_settings", args: &["settings"] },
    Command { name: "start_capture", args: &["mode", "title"] },
    Command { name: "pause_capture", args: &[] },
    Command { name: "resume_capture", args: &[] },
    Command { name: "stop_capture", args: &[] },
    Command { name: "capture_status", args: &[] },
    Command { name: "poll_meetings", args: &[] },
    Command { name: "set_auto_record", args: &["appId", "policy"] },
    Command { name: "ollama_status", args: &[] },
    Command { name: "pull_model", args: &["model"] },
    Command { name: "pull_progress", args: &[] },
    Command { name: "detected_tier", args: &[] },
];

// ---------------------------------------------------------------------------
// Injected platform seams
// ---------------------------------------------------------------------------

/// Where a capture session's audio comes from.
///
/// The one thing B2 replaces: a macOS implementation returns `MacMicSource`
/// (cpal) and `MacSystemSource` (ScreenCaptureKit). Everything above it — the
/// session state machine, the pump thread, the queueing on stop — is this file,
/// and is tested here against [`FakeSources`].
pub trait CaptureSources: Send + Sync {
    /// A fresh microphone source for one recording.
    fn mic(&self) -> Result<Box<dyn AudioSource>>;

    /// A fresh system-audio source for one meeting recording. Returning an
    /// error here is how a platform says "I cannot capture the other side of a
    /// call" — meeting mode then refuses to start rather than silently
    /// recording half a conversation.
    fn system(&self) -> Result<Box<dyn AudioSource>>;
}

/// Scripted sources for tests and for a dev build on a machine with no capture
/// API. Both tracks yield `secs` seconds of tone and then report finished, so a
/// session started with these stops itself — the same path a dead microphone
/// takes, and the one the capture thread has to get right.
pub struct FakeSources {
    pub secs: f64,
}

impl CaptureSources for FakeSources {
    fn mic(&self) -> Result<Box<dyn AudioSource>> {
        Ok(Box::new(FakeSource::tone("microphone", self.secs)))
    }

    fn system(&self) -> Result<Box<dyn AudioSource>> {
        Ok(Box::new(FakeSource::tone("system audio", self.secs)))
    }
}

/// The loaded speech and speaker models the scheduler runs recordings through.
/// Boxed and owned rather than borrowed, because the scheduler thread outlives
/// the call that starts it.
pub struct SchedulerModels {
    pub transcriber: Box<dyn Transcriber + Send + Sync>,
    pub diarizer: Box<dyn Diarizer + Send + Sync>,
}

/// Free space on the volume holding the recordings, read through `sysinfo`.
///
/// The crate ships only `FixedDisk` (a test double), so this is the first real
/// [`DiskSpace`]. It lives here rather than in `capture/` so that module stays
/// free of the process-inspection dependency.
pub struct SysinfoDisk {
    path: PathBuf,
    /// Last reading and when it was taken; see [`DISK_POLL_INTERVAL`].
    cached: Mutex<Option<(Instant, Option<u64>)>>,
}

impl SysinfoDisk {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SysinfoDisk {
            path: path.into(),
            cached: Mutex::new(None),
        }
    }

    /// Megabytes free on the volume that actually holds `path`, chosen as the
    /// mount point that is its longest matching prefix — on macOS the data
    /// volume rather than the read-only system volume, and on any Unix `/` as
    /// the fallback that always matches.
    fn measure(&self) -> Option<u64> {
        let target = existing_ancestor(&self.path)?;
        let disks = sysinfo::Disks::new_with_refreshed_list();
        disks
            .iter()
            .filter(|d| target.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| d.available_space() / 1_048_576)
    }
}

impl DiskSpace for SysinfoDisk {
    fn free_mb(&self) -> Option<u64> {
        let mut cached = lock(&self.cached);
        if let Some((at, value)) = *cached {
            if at.elapsed() < DISK_POLL_INTERVAL {
                return value;
            }
        }
        let value = self.measure();
        *cached = Some((Instant::now(), value));
        value
    }
}

/// The nearest ancestor of `path` that exists, canonicalized. A storage root
/// that has not been created yet still resolves to the volume it will live on.
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut candidate = path;
    loop {
        if let Ok(real) = candidate.canonicalize() {
            return Some(real);
        }
        candidate = candidate.parent()?;
    }
}

/// A [`SystemProbe`] behind an `Arc`, so the runtime can rebuild its
/// [`PowerPolicy`] on every settings change without the caller having to hand
/// over a second probe.
///
/// Needed because `PowerPolicy` is generic over an owned `P: SystemProbe` and
/// the crate provides no `impl SystemProbe for Box<dyn SystemProbe>`.
struct SharedProbe(Arc<dyn SystemProbe + Send + Sync>);

impl SystemProbe for SharedProbe {
    fn read(&self) -> Option<PowerState> {
        self.0.read()
    }
}

/// An [`IdleSource`] that re-reads its decision from a swappable policy on
/// every call.
///
/// `PowerPolicy` holds a *snapshot* of `Settings`, and `scheduler::run_loop`
/// borrows one `&dyn IdleSource` for the entire life of the loop. Without this
/// indirection, a user turning on "only process on wall power" would see no
/// effect until the app restarted — a setting that silently does nothing is
/// worse than one that isn't there. `set_settings` swaps the inner policy and
/// the very next tick gates on it.
struct LivePolicy {
    current: Mutex<Box<dyn IdleSource>>,
}

impl LivePolicy {
    fn new(policy: Box<dyn IdleSource>) -> Self {
        LivePolicy {
            current: Mutex::new(policy),
        }
    }

    fn replace(&self, policy: Box<dyn IdleSource>) {
        *lock(&self.current) = policy;
    }
}

impl IdleSource for LivePolicy {
    fn ok_to_run(&self) -> bool {
        lock(&self.current).ok_to_run()
    }
}

/// Handle on the running scheduler thread: unpark it early, or ask it to stop.
///
/// This is `scheduler::Waker`'s job, but that type has private fields and no
/// constructor, so nothing outside `scheduler.rs` can build one. Rebuilt here
/// rather than reaching into a file this task does not own.
struct SchedulerHandle {
    thread: Thread,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SchedulerHandle {
    /// Cuts the loop's current sleep short so it ticks now.
    fn wake(&self) {
        self.thread.unpark();
    }

    /// Asks the loop to exit after its current tick.
    fn signal_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

// ---------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------

/// Everything the app owns, behind one handle that lives in Tauri's `State`.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    index: Mutex<Index>,
    settings_path: PathBuf,
    /// The live recording, if any. `None` between recordings.
    session: Mutex<Option<Session>>,
    /// Id of the most recently finished recording, so a Stop that races the
    /// disk guard's own stop still hands the UI an id instead of an error.
    last_recording: Mutex<Option<String>>,
    watcher: Mutex<Watcher>,
    sources: Box<dyn CaptureSources>,
    probe: Arc<dyn SystemProbe + Send + Sync>,
    idle: LivePolicy,
    scheduler: Mutex<Option<SchedulerHandle>>,
    /// In-flight and finished model pulls, keyed by model name.
    pulls: Mutex<BTreeMap<String, PullProgress>>,
    /// Free space for `capture_status` between recordings; a live session has
    /// its own.
    disk: SysinfoDisk,
}

impl Runtime {
    /// Opens (or creates) everything the app needs.
    ///
    /// `data_dir` is the app's own directory — Tauri's `app_data_dir()` — and
    /// holds `settings.json` and the disposable search index. Recordings live
    /// under `Settings::storage_root`, falling back to `default_root` the first
    /// time the app runs.
    ///
    /// Changing the storage root in settings takes effect on the next launch:
    /// the scheduler thread and any live session hold the old root, and moving
    /// a library out from under them mid-run is a migration, not a setting.
    pub fn open(
        data_dir: &Path,
        default_root: &Path,
        sources: Box<dyn CaptureSources>,
        probe: Arc<dyn SystemProbe + Send + Sync>,
    ) -> Result<Runtime> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("creating app data dir {}", data_dir.display()))?;

        let settings_path = data_dir.join(SETTINGS_FILE);
        let settings = api::get_settings(&settings_path)?;

        let root = if settings.storage_root.trim().is_empty() {
            default_root.to_path_buf()
        } else {
            PathBuf::from(&settings.storage_root)
        };
        fs::create_dir_all(&root)
            .with_context(|| format!("creating storage root {}", root.display()))?;

        let index = Index::open(&data_dir.join(INDEX_FILE))?;
        let policy = build_policy(&probe, &settings);

        Ok(Runtime {
            inner: Arc::new(Inner {
                disk: SysinfoDisk::new(&root),
                store: Store::new(root),
                index: Mutex::new(index),
                settings_path,
                session: Mutex::new(None),
                last_recording: Mutex::new(None),
                watcher: Mutex::new(Watcher::with_sysinfo()),
                sources,
                probe,
                idle: LivePolicy::new(policy),
                scheduler: Mutex::new(None),
                pulls: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    /// One-time startup work, before the window is shown: recover anything a
    /// crash left mid-flight and refresh the search index from disk.
    ///
    /// Returns `(requeued, indexed)`. The index is a disposable cache derived
    /// from the files, so rebuilding it on launch is also how a deleted or
    /// corrupt database heals itself.
    ///
    /// **Not yet wired:** `capture::recover::recover_orphans`, which repairs a
    /// WAV whose writer died mid-recording, belongs here. It was still a stub
    /// when this was written (Plan B1 task 2, in flight).
    pub fn start_up(&self) -> Result<(usize, usize)> {
        let requeued = requeue_stale(&self.inner.store)?;
        let indexed = lock(&self.inner.index).rebuild(&self.inner.store)?;
        Ok((requeued, indexed))
    }

    // --- library ---------------------------------------------------------

    pub fn list_tasks(&self) -> Result<Vec<String>> {
        api::list_tasks(&self.inner.store)
    }

    pub fn create_task(&self, name: &str) -> Result<()> {
        api::create_task(&self.inner.store, name)
    }

    pub fn list_recordings(&self) -> Result<Vec<RecordingRow>> {
        api::list_recordings(&self.inner.store)
    }

    pub fn get_recording(&self, id: &str) -> Result<RecordingDetail> {
        api::get_recording(&self.inner.store, id)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        api::search(&lock(&self.inner.index), query)
    }

    pub fn update_summary(&self, id: &str, summary_md: &str) -> Result<()> {
        api::update_summary(&self.inner.store, id, summary_md)?;
        // The edited text is what the user will search for next, so re-index
        // rather than waiting for a rebuild.
        let rec = self.inner.find(id)?;
        self.inner.index_one(&rec)
    }

    pub fn assign_task(&self, id: &str, task: &str) -> Result<()> {
        api::assign_task(&self.inner.store, &mut lock(&self.inner.index), id, task)
    }

    /// Renames a recording, moving its directory to match.
    ///
    /// The title is half the on-disk folder name (`2026-07-27 14.30 Standup`),
    /// which is the point of a files-first layout: renaming in the app renames
    /// the folder in Finder. The timestamp half is re-derived from
    /// `meta.created`, so it stays exactly what `Store::create_recording`
    /// wrote.
    ///
    /// Implemented here rather than delegating, because neither `storage` nor
    /// `api` exposes a rename.
    pub fn rename_recording(&self, id: &str, title: &str) -> Result<()> {
        let cleaned = sanitize(title);
        if cleaned.is_empty() {
            bail!("a recording needs a name — that one is empty once punctuation is removed");
        }

        let mut rec = self.inner.find(id)?;
        rec.meta.title = title.to_string();

        // Only the folder name is cosmetic, so a timestamp we cannot parse
        // means "rename in place", never an error that loses the new title.
        if let Some(parent) = rec.dir.parent() {
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&rec.meta.created) {
                let stamp = created.format("%Y-%m-%d %H.%M");
                let dest = unique_dir(parent, &format!("{stamp} {cleaned}"));
                if dest != rec.dir {
                    fs::rename(&rec.dir, &dest).with_context(|| {
                        format!("renaming {} to {}", rec.dir.display(), dest.display())
                    })?;
                    rec.dir = dest;
                }
            }
        }

        self.inner.store.save_meta(&rec)?;
        // The index stores the directory, so skipping this would leave search
        // pointing at a folder that no longer exists.
        self.inner.index_one(&rec)
    }

    pub fn rename_speaker(&self, id: &str, key: &str, name: &str) -> Result<()> {
        api::rename_speaker(&self.inner.store, id, key, name)?;
        let rec = self.inner.find(id)?;
        self.inner.index_one(&rec)
    }

    /// Marks a recording for processing and wakes the scheduler, so "Process
    /// now" means now rather than "within thirty seconds".
    pub fn process_now(&self, id: &str) -> Result<()> {
        api::process_now(&self.inner.store, id)?;
        self.wake_scheduler();
        Ok(())
    }

    // --- settings --------------------------------------------------------

    pub fn get_settings(&self) -> Result<Settings> {
        api::get_settings(&self.inner.settings_path)
    }

    /// Persists settings and rebuilds everything that holds a snapshot of them.
    ///
    /// The rebuild is the point: [`PowerPolicy`] copies `Settings` when it is
    /// built, so without this a user turning on "only process on wall power"
    /// would see no change until the next launch.
    pub fn set_settings(&self, settings: &Settings) -> Result<()> {
        api::set_settings(&self.inner.settings_path, settings)?;
        self.inner.refresh_policy(settings);
        Ok(())
    }

    /// Writes one app's auto-record policy, the way the meeting prompt's
    /// "Always for Zoom" / "Never for Zoom" buttons do.
    pub fn set_auto_record(&self, app_id: &str, policy: AutoRecordPolicy) -> Result<()> {
        let mut settings = self.get_settings()?;
        settings.auto_record.insert(app_id.to_string(), policy);
        self.set_settings(&settings)
    }

    // --- capture ---------------------------------------------------------

    /// Starts recording and returns the first status snapshot.
    ///
    /// Spawns the pump thread that drives `Session::pump`. That loop lives here
    /// rather than in `capture::session` on purpose: `pump` is one synchronous
    /// step, which is what makes the session's own tests free of sleeps and
    /// timing luck.
    pub fn start_capture(&self, mode: Mode, title: &str) -> Result<CaptureStatus> {
        let mut slot = lock(&self.inner.session);
        if slot.is_some() {
            bail!("a recording is already in progress — stop it before starting another");
        }

        let mic = self.inner.sources.mic().context("opening the microphone")?;
        let system = match mode {
            Mode::Meeting => Some(
                self.inner
                    .sources
                    .system()
                    .context("opening system audio for a meeting recording")?,
            ),
            Mode::InPerson => None,
        };

        let mut session = Session::start(
            &self.inner.store,
            mode,
            title,
            mic,
            system,
            Box::new(SysinfoDisk::new(&self.inner.store.root)),
        )?;
        let status = session.status();
        *slot = Some(session);
        *lock(&self.inner.last_recording) = None;
        drop(slot);

        let inner = Arc::clone(&self.inner);
        thread::Builder::new()
            .name("notetaker-capture".to_string())
            .spawn(move || pump_until_done(&inner))
            .context("starting the capture thread")?;

        Ok(status)
    }

    pub fn pause_capture(&self) -> Result<CaptureStatus> {
        let mut slot = lock(&self.inner.session);
        let session = slot.as_mut().context("nothing is being recorded right now")?;
        session.pause();
        Ok(session.status())
    }

    pub fn resume_capture(&self) -> Result<CaptureStatus> {
        let mut slot = lock(&self.inner.session);
        let session = slot.as_mut().context("nothing is being recorded right now")?;
        session.resume();
        Ok(session.status())
    }

    /// Stops the recording, queues it for processing, and returns its id.
    ///
    /// Safe to call after the session already stopped itself (the disk filled,
    /// the mic died): the id of that recording is remembered, because a user
    /// who pressed Stop needs to be taken to their recording either way.
    pub fn stop_capture(&self) -> Result<String> {
        if let Some(id) = self.inner.finish_session()? {
            return Ok(id);
        }
        lock(&self.inner.last_recording)
            .clone()
            .context("nothing is being recorded right now")
    }

    /// A snapshot for the record bar. Cheap enough to poll while recording.
    pub fn capture_status(&self) -> CaptureStatus {
        let mut slot = lock(&self.inner.session);
        match slot.as_mut() {
            Some(session) => session.status(),
            None => CaptureStatus::idle(self.inner.disk.free_mb().unwrap_or(0)),
        }
    }

    /// One step of the capture loop, for a caller that wants to drive capture
    /// itself. The pump thread calls exactly this.
    pub fn pump_once(&self) -> Result<CaptureState> {
        let mut slot = lock(&self.inner.session);
        match slot.as_mut() {
            Some(session) => session.pump(),
            None => Ok(CaptureState::Idle),
        }
    }

    // --- meeting watcher --------------------------------------------------

    /// Drains the debounced meeting events since the last call.
    ///
    /// Settings are re-read from disk each poll, so flipping an app to "never"
    /// silences it on the next tick rather than on the next launch.
    pub fn poll_meetings(&self) -> Result<Vec<MeetingEvent>> {
        let settings = self.get_settings()?;
        Ok(lock(&self.inner.watcher).poll(&settings))
    }

    /// Replaces the meeting watcher — for tests driving a scripted
    /// `FakeProcessSource`, and for a dev build with no meeting apps to watch.
    pub fn set_watcher(&self, watcher: Watcher) {
        *lock(&self.inner.watcher) = watcher;
    }

    // --- local models -----------------------------------------------------

    pub fn ollama_status(&self) -> Result<OllamaStatus> {
        let settings = self.get_settings()?;
        Ok(ollama::status(&settings.llm_base_url, &settings.llm_model))
    }

    /// Starts a model download in the background and returns immediately;
    /// progress arrives through [`Runtime::pull_progress`].
    ///
    /// Returning right away is what lets the first-run screen show a live bar:
    /// a multi-gigabyte pull held open as one invoke would look like a frozen
    /// window.
    pub fn pull_model(&self, model: &str) -> Result<()> {
        let settings = self.get_settings()?;
        let base_url = settings.llm_base_url;
        let model = model.to_string();

        {
            let mut pulls = lock(&self.inner.pulls);
            if pulls.get(&model).is_some_and(|p| !p.done) {
                // Already downloading. Not an error: a user clicking twice must
                // not start a second download of the same model.
                return Ok(());
            }
            pulls.insert(
                model.clone(),
                PullProgress {
                    name: model.clone(),
                    percent: 0.0,
                    error: None,
                    done: false,
                },
            );
        }

        let inner = Arc::clone(&self.inner);
        thread::Builder::new()
            .name("notetaker-pull".to_string())
            .spawn(move || {
                let record = |p: PullProgress| {
                    lock(&inner.pulls).insert(p.name.clone(), p);
                };
                if let Err(e) = ollama::pull(&base_url, &model, record) {
                    log::warn!("pulling {model}: {e:#}");
                    // `pull` reports most failures through the callback, but a
                    // server it cannot reach at all returns before the first
                    // report — and a bar that never reaches `done` is a UI
                    // frozen at 0% with nothing to explain itself.
                    if let Some(progress) = lock(&inner.pulls).get_mut(&model) {
                        if !progress.done {
                            progress.done = true;
                            progress.error = Some(format!("{e:#}"));
                        }
                    }
                }
            })
            .context("starting the model download")?;
        Ok(())
    }

    /// Every pull this session has started, finished ones included, so the UI
    /// can show "done" rather than a bar that vanishes at 100%.
    pub fn pull_progress(&self) -> Vec<PullProgress> {
        lock(&self.inner.pulls).values().cloned().collect()
    }

    /// The hardware tier detected for this machine, as the same string
    /// `Settings::tier_override` accepts.
    pub fn detected_tier(&self) -> String {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        let ram_gb = system.total_memory() / 1_073_741_824;
        tier_name(detect_tier(
            ram_gb,
            cfg!(target_os = "macos") && cfg!(target_arch = "aarch64"),
        ))
        .to_string()
    }

    // --- scheduler --------------------------------------------------------

    /// Spawns the background processing loop.
    ///
    /// The thread owns the loaded models, so they outlive this call — the
    /// reason `scheduler::run_loop` was left for the app layer to wire. The
    /// loop parks between ticks and [`Runtime::process_now`] unparks it, so
    /// "Process now" does not wait out a thirty-second sleep.
    ///
    /// The LLM settings and the task list are read once, when the loop starts:
    /// `run_loop` borrows a single `PipelineDeps` for its whole life. A task
    /// created afterwards is therefore not offered as a suggestion until the
    /// next launch; [`Runtime::tick_once`] has no such limit.
    pub fn start_scheduler(&self, models: SchedulerModels) -> Result<()> {
        let mut slot = lock(&self.inner.scheduler);
        if slot.is_some() {
            bail!("the scheduler is already running");
        }

        let inner = Arc::clone(&self.inner);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let (tx, rx) = mpsc::channel();

        let join = thread::Builder::new()
            .name("notetaker-scheduler".to_string())
            .spawn(move || {
                // The loop's own thread handle is the only way to unpark it,
                // and only the loop can name it.
                let _ = tx.send(thread::current());

                let settings = api::get_settings(&inner.settings_path).unwrap_or_default();
                let llm = LlmClient {
                    base_url: settings.llm_base_url,
                    model: settings.llm_model,
                };
                let deps = PipelineDeps {
                    transcriber: &*models.transcriber,
                    diarizer: &*models.diarizer,
                    llm: &llm,
                    tasks: inner.store.list_tasks().unwrap_or_default(),
                };
                let queue = Queue { store: &inner.store };

                scheduler::run_loop(&queue, &inner.idle, &deps, stop_for_thread, |outcome| {
                    if *outcome == RunOutcome::Ran {
                        // Carry-over I3: without this, a just-processed
                        // recording is invisible to search until a rebuild.
                        if let Err(e) = inner.index_ready() {
                            log::warn!("indexing a finished recording: {e:#}");
                        }
                    }
                });
            })
            .context("starting the scheduler thread")?;

        let thread = rx.recv().context("the scheduler thread never reported in")?;
        *slot = Some(SchedulerHandle {
            thread,
            stop,
            join: Some(join),
        });
        Ok(())
    }

    /// Asks the scheduler to stop and returns without waiting. A recording
    /// already being transcribed finishes first; the queue lives on disk, so
    /// anything it does not reach is picked up next launch.
    pub fn stop_scheduler(&self) {
        if let Some(handle) = lock(&self.inner.scheduler).take() {
            handle.signal_stop();
        }
    }

    /// Stops the scheduler and waits for the thread. Used by tests and by a
    /// graceful quit; blocks for as long as the recording in flight takes.
    pub fn join_scheduler(&self) {
        if let Some(mut handle) = lock(&self.inner.scheduler).take() {
            handle.signal_stop();
            if let Some(join) = handle.join.take() {
                let _ = join.join();
            }
        }
    }

    /// Cuts the scheduler's current sleep short. A no-op when it is not
    /// running.
    fn wake_scheduler(&self) {
        if let Some(handle) = lock(&self.inner.scheduler).as_ref() {
            handle.wake();
        }
    }

    /// One scheduling step on the calling thread — the same decision
    /// `start_scheduler`'s loop makes, including the index refresh, but
    /// synchronous.
    ///
    /// Unlike the loop, this rebuilds `PipelineDeps` each call, so it always
    /// sees the current LLM settings and task list. Tests drive processing
    /// through it so no test depends on a timer.
    pub fn tick_once(&self, models: &SchedulerModels) -> Result<RunOutcome> {
        let settings = self.get_settings()?;
        let llm = LlmClient {
            base_url: settings.llm_base_url,
            model: settings.llm_model,
        };
        let deps = PipelineDeps {
            transcriber: &*models.transcriber,
            diarizer: &*models.diarizer,
            llm: &llm,
            tasks: self.inner.store.list_tasks().unwrap_or_default(),
        };
        let queue = Queue {
            store: &self.inner.store,
        };
        let outcome = scheduler::tick(&queue, &self.inner.idle, &deps)?;
        if outcome == RunOutcome::Ran {
            self.inner.index_ready()?;
        }
        Ok(outcome)
    }

    /// Whether background processing is allowed right now. Feeds the settings
    /// screen's "waiting for the Mac to be idle" line, so the app can say why
    /// nothing is happening instead of looking stuck.
    pub fn idle_ok(&self) -> bool {
        self.inner.idle.ok_to_run()
    }

    /// The storage root in use. The app crate shows it in settings and opens it
    /// in Finder.
    pub fn storage_root(&self) -> &Path {
        &self.inner.store.root
    }
}

impl Inner {
    fn find(&self, id: &str) -> Result<RecordingRef> {
        self.store
            .scan()?
            .into_iter()
            .find(|r| r.meta.id == id)
            .with_context(|| format!("no recording with id {id}"))
    }

    /// Rebuilds the idle/power policy from a fresh `Settings`.
    fn refresh_policy(&self, settings: &Settings) {
        self.idle.replace(build_policy(&self.probe, settings));
    }

    /// Indexes one recording with whatever transcript and summary are on disk.
    fn index_one(&self, rec: &RecordingRef) -> Result<()> {
        let transcript = fs::read_to_string(rec.dir.join(TRANSCRIPT_FILE)).unwrap_or_default();
        let summary = fs::read_to_string(rec.dir.join(SUMMARY_FILE)).unwrap_or_default();
        lock(&self.index).upsert(rec, &transcript, &summary)
    }

    /// Re-indexes every finished recording. Returns how many.
    ///
    /// Carry-over I3. `run_loop`'s callback reports *what happened*, not *which
    /// recording*, so the refresh is a sweep over everything `Ready` rather
    /// than a single upsert. It runs once per completed recording — against a
    /// job that took minutes, re-reading a few dozen small markdown files costs
    /// nothing — and it self-heals a recording that finished while the index
    /// was unwritable.
    fn index_ready(&self) -> Result<usize> {
        let ready: Vec<RecordingRef> = self
            .store
            .scan()?
            .into_iter()
            .filter(|r| r.meta.status == Status::Ready)
            .collect();
        for rec in &ready {
            self.index_one(rec)?;
        }
        Ok(ready.len())
    }

    /// Closes out the live session: finalize the audio, queue the recording,
    /// index it, and remember its id. Returns `None` when there was no session
    /// — the user's Stop raced the disk guard's, and the other one won.
    fn finish_session(&self) -> Result<Option<String>> {
        let mut session = match lock(&self.session).take() {
            Some(session) => session,
            None => return Ok(None),
        };

        // `stop` has already closed the audio files by the time it can report
        // an error, so the recording is never lost to a failed save.
        let id = session.stop()?;
        drop(session);
        *lock(&self.last_recording) = Some(id.clone());

        // TODO(B1 task 2): `capture::flac::finalize_to_flac` goes here, before
        // the enqueue, once that module lands. The pipeline already opens
        // either extension, so nothing else changes.

        let mut rec = self.find(&id)?;
        Queue { store: &self.store }.enqueue(&mut rec)?;
        // Index it now so a brand-new recording is findable by title
        // immediately; the transcript follows when processing finishes.
        self.index_one(&rec)?;
        Ok(Some(id))
    }
}

/// Drives one session to completion, then closes it out. Exits as soon as the
/// session is gone, which is how a user's Stop ends this thread.
fn pump_until_done(inner: &Arc<Inner>) {
    loop {
        let state = {
            let mut slot = lock(&inner.session);
            match slot.as_mut() {
                Some(session) => match session.pump() {
                    Ok(state) => state,
                    Err(e) => {
                        // `pump` only errors when its own stop could not save
                        // meta.json; the audio is already closed on disk.
                        log::warn!("capture pump: {e:#}");
                        CaptureState::Idle
                    }
                },
                // Someone else stopped the recording; nothing left to drive.
                None => return,
            }
        };

        if state == CaptureState::Idle {
            if let Err(e) = inner.finish_session() {
                log::warn!("closing out the recording: {e:#}");
            }
            return;
        }
        thread::sleep(PUMP_INTERVAL);
    }
}

fn build_policy(
    probe: &Arc<dyn SystemProbe + Send + Sync>,
    settings: &Settings,
) -> Box<dyn IdleSource> {
    Box::new(PowerPolicy::new(
        SharedProbe(Arc::clone(probe)),
        settings.clone(),
    ))
}

/// The string form of a hardware tier. Matches `Settings::tier_override`, so
/// "use what was detected" and "force this tier" speak one vocabulary.
fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::AppleSiliconBig => "AppleSiliconBig",
        Tier::AppleSiliconSmall => "AppleSiliconSmall",
        Tier::CpuSmall => "CpuSmall",
    }
}

/// A poisoned lock only means some other thread panicked while holding it.
/// Everything behind these locks is either re-derived from disk or purely
/// advisory, so recovering the guard beats taking the app down with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Strips filesystem-hostile characters. Mirrors `storage::sanitize`, which is
/// private to that module.
fn sanitize(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '/' | ':' | '\\' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// `parent/name`, or `parent/name (2)`, `(3)`, ... if taken. Mirrors
/// `storage::unique_dir`, which is private to that module.
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

// ---------------------------------------------------------------------------
// Contract drift check (carry-over M4)
// ---------------------------------------------------------------------------
//
// Everything below is `#[cfg(test)]`: it exists to police [`COMMANDS`] against
// `src/lib/ipc.ts` on every test run, and reading a TypeScript file is no part
// of what the shipped app does.

/// One `invoke(...)` call parsed out of `src/lib/ipc.ts`.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Invocation {
    command: String,
    args: Vec<String>,
}

/// Finds every `invoke<T>("name", { arg, arg })` in a TypeScript source.
///
/// Deliberately a scanner rather than a regex: the crate has no regex
/// dependency, and the shapes involved — nested generics like
/// `Record<string, string>`, multi-line argument objects — are exactly what a
/// regex gets wrong quietly.
#[cfg(test)]
fn extract_invocations(source: &str) -> Vec<Invocation> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if !starts_word(&chars, i, "invoke") {
            i += 1;
            continue;
        }

        let mut j = skip_ws(&chars, i + "invoke".len());
        // Optional type argument: `invoke<RecordingRow[]>(...)`.
        if chars.get(j) == Some(&'<') {
            match balanced(&chars, j, '<', '>') {
                Some(end) => j = skip_ws(&chars, end),
                None => {
                    i += 1;
                    continue;
                }
            }
        }
        // `import { invoke } from ...` lands here and is skipped.
        if chars.get(j) != Some(&'(') {
            i += 1;
            continue;
        }
        j = skip_ws(&chars, j + 1);
        if chars.get(j) != Some(&'"') {
            i += 1;
            continue;
        }

        let start = j + 1;
        let Some(end) = (start..chars.len()).find(|&k| chars[k] == '"') else {
            break;
        };
        let command: String = chars[start..end].iter().collect();

        j = skip_ws(&chars, end + 1);
        let mut args = Vec::new();
        if chars.get(j) == Some(&',') {
            j = skip_ws(&chars, j + 1);
            if chars.get(j) == Some(&'{') {
                if let Some(close) = balanced(&chars, j, '{', '}') {
                    let body: String = chars[j + 1..close - 1].iter().collect();
                    args = split_arg_names(&body);
                    j = close;
                }
            }
        }

        out.push(Invocation { command, args });
        i = j.max(i + 1);
    }
    out
}

/// Whether `word` starts at `i` and is not the tail of a longer identifier.
#[cfg(test)]
fn starts_word(chars: &[char], i: usize, word: &str) -> bool {
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '$') {
        return false;
    }
    if chars.len() - i < word.len() {
        return false;
    }
    chars[i..i + word.len()].iter().copied().eq(word.chars())
}

#[cfg(test)]
fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

/// Index just past the `close` that matches the `open` at `i`.
#[cfg(test)]
fn balanced(chars: &[char], i: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (k, &c) in chars.iter().enumerate().skip(i) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(k + 1);
            }
        }
    }
    None
}

/// Argument names out of an object literal's body: `id, summaryMd` and
/// `id: theId` both yield the key.
#[cfg(test)]
fn split_arg_names(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in body.chars() {
        match c {
            '{' | '[' | '(' => {
                depth += 1;
                current.push(c);
            }
            '}' | ']' | ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                push_arg_name(&mut names, &current);
                current.clear();
            }
            _ => current.push(c),
        }
    }
    push_arg_name(&mut names, &current);
    names
}

#[cfg(test)]
fn push_arg_name(names: &mut Vec<String>, piece: &str) {
    let name = piece.split(':').next().unwrap_or("").trim();
    if !name.is_empty() {
        names.push(name.to_string());
    }
}

/// Every way `found` (what the UI actually sends) and `table` (what this file
/// documents) disagree, in sentences a reader can act on. Empty means the
/// contract holds.
#[cfg(test)]
fn contract_problems(found: &[Invocation], table: &[Command]) -> Vec<String> {
    let mut problems = Vec::new();

    for call in found {
        match table.iter().find(|c| c.name == call.command) {
            None => problems.push(format!(
                "ipc.ts invokes \"{}\", which is not in runtime::COMMANDS",
                call.command
            )),
            Some(command) => {
                let documented: Vec<&str> = command.args.to_vec();
                let sent: Vec<&str> = call.args.iter().map(String::as_str).collect();
                if documented != sent {
                    problems.push(format!(
                        "\"{}\" arguments drifted: ipc.ts sends {sent:?}, \
                         runtime::COMMANDS says {documented:?}",
                        call.command
                    ));
                }
            }
        }
    }

    for command in table {
        if !found.iter().any(|c| c.command == command.name) {
            problems.push(format!(
                "runtime::COMMANDS documents \"{}\", which ipc.ts never invokes",
                command.name
            ));
        }
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::diarize::SpeakerSpan;
    use crate::power::probe::FakeProbe;
    use crate::watch::watcher::FakeProcessSource;

    /// Anything a runtime test waits on has to happen inside this, or the test
    /// fails with a message rather than hanging.
    const PATIENCE: Duration = Duration::from_secs(20);

    // --- stub models -----------------------------------------------------

    /// Pipeline stubs in the same style as `scheduler.rs`'s: enough for
    /// `process_recording` to run end to end without loading real models.
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

    struct FixedText(&'static str);
    impl Transcriber for FixedText {
        fn transcribe(&self, _: &[f32], _: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>> {
            Ok(vec![(0.0, 1.0, self.0.to_string())])
        }
    }

    fn models(text: &'static str) -> SchedulerModels {
        SchedulerModels {
            transcriber: Box::new(FixedText(text)),
            diarizer: Box::new(OneSpeaker),
        }
    }

    /// An Ollama stand-in that answers both LLM calls, so a recording can go
    /// all the way to `Ready` without a real model.
    ///
    /// One mock serves summarize *and* suggest-task: the reply is the
    /// suggestion JSON `suggest_task` parses, which summarize stores verbatim
    /// as the summary. An empty task name is the model saying "none of these
    /// fit", which is what an unsorted recording should get.
    fn fake_llm() -> httpmock::MockServer {
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant",
                    "content": "{\"task\": \"\", \"confidence\": 0.1}" }
            }));
        });
        server
    }

    // --- harness ---------------------------------------------------------

    /// A runtime on a temp dir, with fake audio and a machine that reads as
    /// idle and plugged in.
    fn runtime(dir: &Path, capture_secs: f64) -> Runtime {
        runtime_with_probe(
            dir,
            capture_secs,
            Some(PowerState {
                idle_secs: 9_000,
                on_ac: true,
                battery_pct: Some(90),
            }),
        )
    }

    fn runtime_with_probe(dir: &Path, capture_secs: f64, state: Option<PowerState>) -> Runtime {
        Runtime::open(
            &dir.join("app"),
            &dir.join("Notetaker"),
            Box::new(FakeSources { secs: capture_secs }),
            Arc::new(FakeProbe { state }),
        )
        .expect("runtime must open on a fresh directory")
    }

    /// Points the runtime's LLM at `base_url` and relaxes the idle gate, which
    /// is what every processing test needs.
    fn use_llm(rt: &Runtime, base_url: &str) {
        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = base_url.to_string();
        settings.llm_model = "test".to_string();
        settings.min_idle_secs = 0;
        rt.set_settings(&settings).unwrap();
    }

    /// Polls `condition` until it holds or [`PATIENCE`] runs out. No
    /// unconditional sleeps anywhere in this module: a bug shows up as a
    /// failure with a message, never as a hung suite.
    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if condition() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn status_of(rt: &Runtime, id: &str) -> Status {
        rt.list_recordings()
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.status)
            .expect("recording must exist")
    }

    fn attempts(rt: &Runtime, id: &str) -> u32 {
        rt.inner
            .store
            .scan()
            .unwrap()
            .into_iter()
            .find(|r| r.meta.id == id)
            .map(|r| r.meta.attempts)
            .unwrap_or(0)
    }

    /// Records `secs` of fake audio end to end and returns the recording id.
    fn record(rt: &Runtime, mode: Mode, title: &str) -> String {
        rt.start_capture(mode, title).unwrap();
        // The fake sources run dry, so the session stops itself — the same path
        // a dead microphone takes.
        wait_until("capture to finish", || {
            rt.capture_status().state == CaptureState::Idle
        });
        rt.stop_capture().unwrap()
    }

    // --- the headline: capture -> queue -> process -> search ---------------

    #[test]
    fn a_recording_captured_and_processed_is_findable_by_a_word_in_its_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.3);
        let server = fake_llm();
        use_llm(&rt, &server.base_url());

        let id = record(&rt, Mode::InPerson, "Lecture 3");
        assert_eq!(
            status_of(&rt, &id),
            Status::Queued,
            "stopping a recording must queue it for processing"
        );
        assert!(
            !rt.search("photosynthesis")
                .unwrap()
                .iter()
                .any(|h| h.id == id),
            "nothing is transcribed yet, so the word cannot be findable"
        );

        let outcome = rt
            .tick_once(&models("we covered photosynthesis today"))
            .unwrap();
        assert_eq!(outcome, RunOutcome::Ran);
        assert_eq!(status_of(&rt, &id), Status::Ready);

        let hits = rt.search("photosynthesis").unwrap();
        assert_eq!(hits.len(), 1, "expected exactly one hit, got {hits:?}");
        assert_eq!(hits[0].id, id);
        assert_eq!(hits[0].title, "Lecture 3");

        let detail = rt.get_recording(&id).unwrap();
        assert!(detail.transcript_md.contains("photosynthesis"));
        assert!(!detail.summary_md.is_empty());
    }

    /// Carry-over I3, as its own regression: a fresh recording used to be
    /// invisible to search until the whole index was rebuilt.
    #[test]
    fn a_just_processed_recording_is_searchable_with_no_rebuild_in_between() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let server = fake_llm();
        use_llm(&rt, &server.base_url());

        let id = record(&rt, Mode::InPerson, "Untitled");
        rt.tick_once(&models("the quarterly amortisation schedule"))
            .unwrap();

        // Deliberately no `start_up()` / `Index::rebuild` anywhere after
        // processing — the runtime must have indexed it on the success path.
        let hits = rt.search("amortisation").unwrap();
        assert_eq!(hits.len(), 1, "expected the new recording, got {hits:?}");
        assert_eq!(hits[0].id, id);
    }

    #[test]
    fn a_meeting_recording_captures_both_tracks_and_processes() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let server = fake_llm();
        use_llm(&rt, &server.base_url());

        let id = record(&rt, Mode::Meeting, "Standup");
        let rec_dir = rt.inner.find(&id).unwrap().dir;
        assert!(rec_dir.join("audio-mic.wav").exists());
        assert!(
            rec_dir.join("audio-system.wav").exists(),
            "a meeting must capture the other side of the call"
        );

        assert_eq!(rt.tick_once(&models("hello")).unwrap(), RunOutcome::Ran);
        assert_eq!(status_of(&rt, &id), Status::Ready);
    }

    // --- carry-over I4: the scheduler thread and its wake ------------------

    #[test]
    fn process_now_wakes_a_parked_scheduler_instead_of_waiting_out_its_sleep() {
        // The loop sleeps `scheduler::TICK_INTERVAL` (30s) between ticks, and
        // this test's whole patience is 20s — so a second tick arriving at all
        // proves the unpark, with no dependence on how fast anything runs.
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        // A dead port: processing fails fast at the summarize step, which
        // increments `attempts` — a counter the test can watch tick up.
        use_llm(&rt, "http://127.0.0.1:1");

        let id = record(&rt, Mode::InPerson, "Lecture");
        rt.start_scheduler(models("hello")).unwrap();

        // The loop ticks once on entry, fails, and parks for thirty seconds.
        wait_until("the scheduler's first tick", || attempts(&rt, &id) >= 1);
        assert_eq!(status_of(&rt, &id), Status::Queued, "queued for a retry");

        rt.process_now(&id).unwrap();
        wait_until("the woken scheduler's second tick", || {
            attempts(&rt, &id) >= 2
        });

        rt.join_scheduler();
    }

    #[test]
    fn the_scheduler_thread_processes_a_queued_recording_and_indexes_it() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let server = fake_llm();
        use_llm(&rt, &server.base_url());

        let id = record(&rt, Mode::InPerson, "Lecture");
        rt.start_scheduler(models("mitochondria are the powerhouse"))
            .unwrap();

        wait_until("the scheduler to finish the recording", || {
            status_of(&rt, &id) == Status::Ready
        });
        wait_until("the index to catch up", || {
            rt.search("mitochondria").unwrap().len() == 1
        });

        rt.join_scheduler();
    }

    #[test]
    fn starting_the_scheduler_twice_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        rt.start_scheduler(models("hello")).unwrap();
        let err = rt.start_scheduler(models("hello")).unwrap_err();
        assert!(format!("{err:#}").contains("already running"), "{err:#}");
        rt.join_scheduler();
    }

    // --- settings change rebuilds the power policy ------------------------

    #[test]
    fn set_settings_rebuilds_the_power_policy_so_require_ac_takes_effect_at_once() {
        // The bug this guards: `PowerPolicy` copies `Settings` when it is
        // built, so a runtime that never rebuilt it would leave "only process
        // on wall power" doing nothing until the next launch.
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime_with_probe(
            dir.path(),
            0.2,
            Some(PowerState {
                idle_secs: 9_000,
                on_ac: false, // on battery
                battery_pct: Some(90),
            }),
        );
        let server = fake_llm();

        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = server.base_url();
        settings.llm_model = "test".to_string();
        settings.min_idle_secs = 0;
        settings.require_ac = false;
        rt.set_settings(&settings).unwrap();
        assert!(rt.idle_ok(), "on battery is fine while require_ac is off");

        let id = record(&rt, Mode::InPerson, "Lecture");

        settings.require_ac = true;
        rt.set_settings(&settings).unwrap();
        assert!(!rt.idle_ok(), "the new setting must apply immediately");
        assert_eq!(
            rt.tick_once(&models("hello")).unwrap(),
            RunOutcome::NotIdle,
            "processing must stop the moment the user asks for wall power"
        );
        assert_eq!(status_of(&rt, &id), Status::Queued);

        settings.require_ac = false;
        rt.set_settings(&settings).unwrap();
        assert_eq!(rt.tick_once(&models("hello")).unwrap(), RunOutcome::Ran);
        assert_eq!(status_of(&rt, &id), Status::Ready);
    }

    // --- capture controls -------------------------------------------------

    #[test]
    fn capture_status_between_recordings_is_idle_with_a_real_disk_reading() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);

        let status = rt.capture_status();
        assert_eq!(status.state, CaptureState::Idle);
        assert_eq!(status.recording_id, None);
        assert_eq!(status.elapsed_s, 0.0);
        assert!(
            status.disk_free_mb > 0,
            "free space on the storage volume must be readable, got {}",
            status.disk_free_mb
        );
    }

    #[test]
    fn a_second_start_while_recording_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        // Long enough that the first session is certainly still live.
        let rt = runtime(dir.path(), 600.0);

        rt.start_capture(Mode::InPerson, "Lecture").unwrap();
        let err = rt.start_capture(Mode::InPerson, "Another").unwrap_err();
        assert!(format!("{err:#}").contains("already in progress"), "{err:#}");

        rt.stop_capture().unwrap();
        assert_eq!(rt.capture_status().state, CaptureState::Idle);
    }

    #[test]
    fn pause_and_resume_report_the_state_the_record_bar_renders() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 600.0);

        assert_eq!(
            rt.start_capture(Mode::InPerson, "Lecture").unwrap().state,
            CaptureState::Recording
        );
        assert_eq!(rt.pause_capture().unwrap().state, CaptureState::Paused);
        assert_eq!(rt.resume_capture().unwrap().state, CaptureState::Recording);
        rt.stop_capture().unwrap();
    }

    #[test]
    fn pause_and_stop_with_nothing_recording_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);

        for err in [
            rt.pause_capture().unwrap_err(),
            rt.resume_capture().unwrap_err(),
            rt.stop_capture().unwrap_err(),
        ] {
            assert!(
                format!("{err:#}").contains("nothing is being recorded"),
                "{err:#}"
            );
        }
    }

    #[test]
    fn a_session_that_stops_itself_is_still_queued_and_still_returns_its_id() {
        // The disk-guard path: nobody pressed Stop, so if the pump thread did
        // not queue the recording it would sit at `Recorded` forever.
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);

        rt.start_capture(Mode::InPerson, "Lecture").unwrap();
        wait_until("the recording to be queued by the capture thread", || {
            rt.list_recordings()
                .map(|rows| rows.iter().any(|r| r.status == Status::Queued))
                .unwrap_or(false)
        });

        let id = rt
            .stop_capture()
            .expect("Stop after a self-stop must still work");
        assert_eq!(status_of(&rt, &id), Status::Queued);
    }

    #[test]
    fn a_new_recording_is_findable_by_title_before_it_is_processed() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);

        let id = record(&rt, Mode::InPerson, "Thermodynamics");
        let hits = rt.search("Thermodynamics").unwrap();
        assert_eq!(hits.len(), 1, "got {hits:?}");
        assert_eq!(hits[0].id, id);
    }

    // --- library commands -------------------------------------------------

    #[test]
    fn rename_recording_moves_the_folder_and_keeps_search_pointing_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);

        let id = record(&rt, Mode::InPerson, "Untitled");
        let before = rt.inner.find(&id).unwrap().dir;

        rt.rename_recording(&id, "Accounting 302 — midterm review")
            .unwrap();

        let after = rt.inner.find(&id).unwrap().dir;
        assert_ne!(before, after, "the folder name carries the title");
        assert!(!before.exists(), "the old folder must not be left behind");
        assert!(
            after
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("Accounting 302 — midterm review"),
            "{}",
            after.display()
        );
        assert_eq!(
            rt.get_recording(&id).unwrap().title,
            "Accounting 302 — midterm review"
        );

        let hits = rt.search("midterm").unwrap();
        assert_eq!(hits.len(), 1, "the renamed recording must stay findable");
        assert_eq!(hits[0].id, id);
    }

    #[test]
    fn rename_recording_refuses_a_title_that_is_only_punctuation() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let id = record(&rt, Mode::InPerson, "Lecture");

        let err = rt.rename_recording(&id, " /// ").unwrap_err();
        assert!(format!("{err:#}").contains("needs a name"), "{err:#}");
        assert_eq!(rt.get_recording(&id).unwrap().title, "Lecture");
    }

    #[test]
    fn tasks_summaries_and_speakers_all_delegate_to_the_api_layer() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);

        assert_eq!(rt.list_tasks().unwrap(), Vec::<String>::new());
        rt.create_task("Accounting 302").unwrap();
        assert_eq!(rt.list_tasks().unwrap(), vec!["Accounting 302".to_string()]);

        let id = record(&rt, Mode::InPerson, "Lecture");
        rt.update_summary(&id, "## TL;DR\ndepreciation schedules")
            .unwrap();
        assert_eq!(
            rt.get_recording(&id).unwrap().summary_md,
            "## TL;DR\ndepreciation schedules"
        );
        assert_eq!(
            rt.search("depreciation").unwrap().len(),
            1,
            "an edited summary must be searchable without a rebuild"
        );

        rt.assign_task(&id, "Accounting 302").unwrap();
        assert_eq!(
            rt.get_recording(&id).unwrap().task.as_deref(),
            Some("Accounting 302")
        );

        rt.rename_speaker(&id, "spk1", "Jamie").unwrap();
        assert_eq!(
            rt.get_recording(&id).unwrap().speakers.get("spk1"),
            Some(&"Jamie".to_string())
        );
    }

    #[test]
    fn process_now_queues_a_finished_recording_again() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let server = fake_llm();
        use_llm(&rt, &server.base_url());

        let id = record(&rt, Mode::InPerson, "Lecture");
        rt.tick_once(&models("hello")).unwrap();
        assert_eq!(status_of(&rt, &id), Status::Ready);

        // No scheduler running: `process_now` must still mark it, and must not
        // fail for want of a thread to wake.
        rt.process_now(&id).unwrap();
        assert_eq!(status_of(&rt, &id), Status::Queued);
    }

    #[test]
    fn start_up_recovers_a_recording_a_crash_left_processing() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let id = record(&rt, Mode::InPerson, "Lecture");

        let mut rec = rt.inner.find(&id).unwrap();
        rec.meta.status = Status::Processing;
        rt.inner.store.save_meta(&rec).unwrap();

        let (requeued, indexed) = rt.start_up().unwrap();
        assert_eq!(requeued, 1);
        assert_eq!(indexed, 1);
        assert_eq!(status_of(&rt, &id), Status::Queued);
    }

    // --- settings and the watcher -----------------------------------------

    #[test]
    fn settings_round_trip_through_the_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);

        let mut settings = rt.get_settings().unwrap();
        settings.llm_model = "qwen3:14b".to_string();
        settings.keep_wav = true;
        settings.min_idle_secs = 60;
        rt.set_settings(&settings).unwrap();

        let round_tripped = rt.get_settings().unwrap();
        assert_eq!(round_tripped.llm_model, "qwen3:14b");
        assert!(round_tripped.keep_wav);
        assert_eq!(round_tripped.min_idle_secs, 60);
    }

    #[test]
    fn set_auto_record_persists_one_apps_policy_without_touching_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);

        rt.set_auto_record("zoom", AutoRecordPolicy::Always).unwrap();
        rt.set_auto_record("slack", AutoRecordPolicy::Never).unwrap();

        let settings = rt.get_settings().unwrap();
        assert_eq!(
            settings.auto_record,
            BTreeMap::from([
                ("zoom".to_string(), AutoRecordPolicy::Always),
                ("slack".to_string(), AutoRecordPolicy::Never),
            ])
        );
    }

    #[test]
    fn poll_meetings_reports_a_debounced_start_and_applies_the_saved_policy() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        rt.set_auto_record("zoom", AutoRecordPolicy::Always).unwrap();

        let frames = vec![vec!["zoom.us".to_string()]; 4];
        rt.set_watcher(Watcher::new(Box::new(FakeProcessSource::new(frames))));

        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(rt.poll_meetings().unwrap());
        }

        assert_eq!(events.len(), 1, "one debounced start: {events:?}");
        assert_eq!(events[0].app_id, "zoom");
        assert!(
            events[0].auto_start,
            "the saved policy must reach the event the UI acts on"
        );
    }

    #[test]
    fn poll_meetings_is_quiet_on_a_machine_with_no_meeting_apps() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        rt.set_watcher(Watcher::new(Box::new(FakeProcessSource::new(vec![]))));
        assert!(rt.poll_meetings().unwrap().is_empty());
    }

    // --- local models ------------------------------------------------------

    #[test]
    fn ollama_status_reads_the_configured_server() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/tags");
            then.status(200)
                .json_body(serde_json::json!({"models": [{"name": "qwen3:8b"}]}));
        });

        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = server.base_url();
        settings.llm_model = "qwen3:8b".to_string();
        rt.set_settings(&settings).unwrap();

        let status = rt.ollama_status().unwrap();
        assert!(status.running);
        assert!(status.model_ready);
    }

    #[test]
    fn ollama_status_on_a_dead_port_reports_not_running_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = "http://127.0.0.1:1".to_string();
        rt.set_settings(&settings).unwrap();

        let status = rt.ollama_status().unwrap();
        assert!(!status.running);
        assert!(!status.model_ready);
    }

    #[test]
    fn pull_model_runs_in_the_background_and_reports_progress_to_completion() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/api/pull");
            then.status(200).body(
                "{\"status\":\"downloading\",\"total\":1000,\"completed\":500}\n\
                 {\"status\":\"success\"}",
            );
        });

        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = server.base_url();
        rt.set_settings(&settings).unwrap();

        rt.pull_model("qwen3:8b").unwrap();
        assert_eq!(
            rt.pull_progress().len(),
            1,
            "the bar must appear the moment the pull is asked for"
        );

        wait_until("the pull to finish", || {
            rt.pull_progress().first().is_some_and(|p| p.done)
        });
        let progress = rt.pull_progress();
        assert_eq!(progress[0].name, "qwen3:8b");
        assert_eq!(progress[0].percent, 100.0);
        assert_eq!(progress[0].error, None);
    }

    #[test]
    fn a_failed_pull_leaves_an_error_the_ui_can_show_instead_of_a_frozen_bar() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = "http://127.0.0.1:1".to_string();
        rt.set_settings(&settings).unwrap();

        rt.pull_model("qwen3:8b").unwrap();
        wait_until("the failed pull to report", || {
            rt.pull_progress().first().is_some_and(|p| p.done)
        });
        assert!(rt.pull_progress()[0].error.is_some());
    }

    #[test]
    fn detected_tier_is_one_of_the_names_settings_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.1);
        let tier = rt.detected_tier();
        assert!(
            ["AppleSiliconBig", "AppleSiliconSmall", "CpuSmall"].contains(&tier.as_str()),
            "unknown tier {tier}"
        );
        // There is no Apple Silicon on this build box to detect.
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert_eq!(tier, "CpuSmall");
    }

    #[test]
    fn the_real_disk_probe_reads_free_space_for_a_path_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let disk = SysinfoDisk::new(dir.path().join("not").join("created").join("yet"));
        let free = disk.free_mb();
        assert!(
            free.is_some_and(|mb| mb > 0),
            "the storage volume must be measurable before the root is created, got {free:?}"
        );
        // Second read comes from the cache; same answer, no second enumeration.
        assert_eq!(disk.free_mb(), free);
    }

    // --- every command is reachable ----------------------------------------

    #[test]
    fn every_command_in_the_table_is_reachable_on_the_runtime() {
        // The done-criterion for this task: the app crate's wrappers must have
        // nothing left to do but delegate. A row added to `COMMANDS` with no
        // method behind it fails right here.
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), 0.2);
        let mut settings = rt.get_settings().unwrap();
        settings.llm_base_url = "http://127.0.0.1:1".to_string();
        rt.set_settings(&settings).unwrap();
        rt.set_watcher(Watcher::new(Box::new(FakeProcessSource::new(vec![]))));

        let id = record(&rt, Mode::InPerson, "Lecture");

        let fallible: Vec<(&str, Result<()>)> = vec![
            ("list_tasks", rt.list_tasks().map(|_| ())),
            ("create_task", rt.create_task("Accounting 302")),
            ("list_recordings", rt.list_recordings().map(|_| ())),
            ("get_recording", rt.get_recording(&id).map(|_| ())),
            ("search", rt.search("lecture").map(|_| ())),
            ("update_summary", rt.update_summary(&id, "## TL;DR\nhi")),
            ("assign_task", rt.assign_task(&id, "Accounting 302")),
            ("rename_recording", rt.rename_recording(&id, "Lecture 3")),
            ("rename_speaker", rt.rename_speaker(&id, "spk1", "Jamie")),
            ("process_now", rt.process_now(&id)),
            ("get_settings", rt.get_settings().map(|_| ())),
            ("set_settings", rt.set_settings(&settings)),
            (
                "set_auto_record",
                rt.set_auto_record("zoom", AutoRecordPolicy::Ask),
            ),
            ("poll_meetings", rt.poll_meetings().map(|_| ())),
            ("ollama_status", rt.ollama_status().map(|_| ())),
            ("pull_model", rt.pull_model("qwen3:8b")),
        ];
        let called: Vec<&str> = fallible
            .into_iter()
            .map(|(name, result)| {
                result.unwrap_or_else(|e| panic!("{name} failed: {e:#}"));
                name
            })
            .collect();

        // The infallible ones, plus the capture lifecycle.
        let _ = rt.capture_status();
        let _ = rt.pull_progress();
        let _ = rt.detected_tier();
        assert!(rt.pause_capture().is_err(), "nothing is recording");
        assert!(rt.resume_capture().is_err(), "nothing is recording");
        rt.start_capture(Mode::InPerson, "Another").unwrap();
        rt.stop_capture().unwrap();

        // capture_status, pull_progress, detected_tier, pause, resume, start,
        // stop — seven not in the list above.
        assert_eq!(
            called.len() + 7,
            COMMANDS.len(),
            "every command in COMMANDS must be exercised here; called {called:?}"
        );
    }

    // --- carry-over M4: the contract drift check ---------------------------

    fn ipc_ts() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/ipc.ts");
        fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading the UI contract at {}: {e}", path.display()))
    }

    #[test]
    fn ipc_contract_matches_the_documented_command_table() {
        // The bug this exists to catch is invisible: a renamed argument makes
        // the invoke fail at runtime with a deserialization error the user
        // reads as "the button does nothing".
        let found = extract_invocations(&ipc_ts());
        assert!(
            !found.is_empty(),
            "the ipc.ts scanner found no invocations at all — the scanner is broken, \
             not the contract"
        );

        let problems = contract_problems(&found, COMMANDS);
        assert!(
            problems.is_empty(),
            "src/lib/ipc.ts and runtime::COMMANDS have drifted:\n  {}",
            problems.join("\n  ")
        );
    }

    #[test]
    fn every_documented_argument_name_is_camel_case() {
        // Tauri sends the JS object's keys straight through, so a snake_case
        // name here would mean the UI and the wrapper disagree. The wrappers
        // therefore carry `rename_all = "camelCase"`.
        for command in COMMANDS {
            for arg in command.args {
                assert!(
                    !arg.contains('_') && !arg.starts_with(char::is_uppercase),
                    "{}'s argument {arg:?} is not camelCase",
                    command.name
                );
            }
        }
    }

    #[test]
    fn the_contract_check_fails_when_either_side_drifts() {
        // Proof that the test above can fail: every drift a human could
        // introduce, against a two-command table.
        let table = &[
            Command {
                name: "update_summary",
                args: &["id", "summaryMd"],
            },
            Command {
                name: "capture_status",
                args: &[],
            },
        ];
        let good = r#"
            updateSummary: (id: string, summaryMd: string) =>
                invoke<void>("update_summary", { id, summaryMd }),
            captureStatus: () => invoke<CaptureStatus>("capture_status"),
        "#;
        assert!(contract_problems(&extract_invocations(good), table).is_empty());

        let cases = [
            (
                "renamed argument",
                r#"invoke<void>("update_summary", { id, summary_md });
                   invoke<CaptureStatus>("capture_status");"#,
                "drifted",
            ),
            (
                "dropped argument",
                r#"invoke<void>("update_summary", { id });
                   invoke<CaptureStatus>("capture_status");"#,
                "drifted",
            ),
            (
                "renamed command",
                r#"invoke<void>("updateSummary", { id, summaryMd });
                   invoke<CaptureStatus>("capture_status");"#,
                "not in runtime::COMMANDS",
            ),
            (
                "command the UI stopped calling",
                r#"invoke<void>("update_summary", { id, summaryMd });"#,
                "never invokes",
            ),
        ];
        for (name, source, expected) in cases {
            let problems = contract_problems(&extract_invocations(source), table);
            assert!(
                problems.iter().any(|p| p.contains(expected)),
                "{name}: expected a problem mentioning {expected:?}, got {problems:?}"
            );
        }
    }

    #[test]
    fn the_ipc_scanner_handles_every_shape_the_contract_file_uses() {
        let source = r#"
            import { invoke } from "@tauri-apps/api/core";
            // A comment mentioning invoke, with an em dash — and no call.
            noArgs: () => invoke<string[]>("list_tasks"),
            oneArg: (id: string) => invoke<RecordingDetail>("get_recording", { id }),
            generic: () => invoke<Record<string, string>>("speakers", { id }),
            multiline: (id: string, summaryMd: string) =>
                invoke<void>("update_summary", {
                    id,
                    summaryMd,
                }),
            renamed: (theId: string) => invoke<void>("process_now", { id: theId }),
            untyped: () => invoke("legacy_command"),
        "#;

        let seen: Vec<(&str, Vec<&str>)> = extract_invocations(source)
            .iter()
            .map(|i| {
                (
                    // Leaked only so the comparison below reads as literals.
                    Box::leak(i.command.clone().into_boxed_str()) as &str,
                    i.args
                        .iter()
                        .map(|a| Box::leak(a.clone().into_boxed_str()) as &str)
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                ("list_tasks", vec![]),
                ("get_recording", vec!["id"]),
                ("speakers", vec!["id"]),
                ("update_summary", vec!["id", "summaryMd"]),
                ("process_now", vec!["id"]),
                ("legacy_command", vec![]),
            ],
            "the import line must be skipped and every call form recognised"
        );
    }

    #[test]
    fn the_command_table_has_no_duplicates() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate command in COMMANDS");
    }
}
