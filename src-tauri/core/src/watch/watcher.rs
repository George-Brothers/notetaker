//! Polling for meeting apps, debouncing, and applying the per-app policy.
//!
//! The watcher is a pure state machine over a list of process names. Everything
//! platform-specific lives in [`ProcessSource`], which has a `sysinfo` impl for
//! production and a scripted fake for tests — so the debounce, the
//! never-fire-twice rule, and the policy resolution are all provable here on
//! Linux, with no timers and no sleeps in the test suite.
//!
//! Time is counted in polls, not seconds: the caller owns the interval. That
//! keeps the whole module deterministic — a test drives ten "minutes" of
//! timeline in microseconds by calling [`Watcher::poll`] ten times.

use std::collections::HashSet;
use std::sync::Mutex;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::api::Settings;
use crate::watch::apps::{app_for_process, KNOWN_APPS};
use crate::watch::{AutoRecordPolicy, MeetingEvent, MeetingEventKind};

/// Consecutive polls an app must be present before `Started` fires, and absent
/// before `Ended` does.
///
/// Three, against the app's ~5s poll interval, is ~15 seconds of confirmation:
/// long enough that a helper process blinking in and out never produces a
/// prompt, short enough that the prompt still arrives while the user is looking
/// at the "join meeting" screen. A false prompt costs more than fifteen
/// seconds of latency, so when in doubt this number goes up, not down.
pub const CONFIRM_POLLS: u32 = 3;

/// Where the running-process list comes from. The one seam between the
/// watcher's logic and the operating system.
pub trait ProcessSource: Send + Sync {
    /// Process names only — no pids, no paths. The watcher needs nothing else,
    /// and keeping the surface this small is what makes the fake trivial.
    fn running_processes(&self) -> Vec<String>;
}

/// The production source: `sysinfo`, which enumerates processes identically on
/// macOS and Linux.
///
/// Holds one `System` across polls (behind a `Mutex`, because the trait takes
/// `&self` and `Watcher` has to be `Sync` to live in Tauri state) so each poll
/// is an incremental refresh rather than a fresh scan of every process.
pub struct SysinfoSource {
    system: Mutex<System>,
}

impl SysinfoSource {
    pub fn new() -> Self {
        SysinfoSource {
            system: Mutex::new(System::new()),
        }
    }
}

impl Default for SysinfoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSource for SysinfoSource {
    fn running_processes(&self) -> Vec<String> {
        // A poisoned lock only means some other thread panicked mid-refresh;
        // the process map is still readable, and a watcher that panics the app
        // because a poll went wrong would be worse than a stale list.
        let mut system = self.system.lock().unwrap_or_else(|e| e.into_inner());
        // `nothing()` is deliberate: process names come for free, and asking
        // for cpu/memory/disk on every poll would make a background watcher
        // expensive.
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        system
            .processes()
            .values()
            .map(|p| p.name().to_string_lossy().into_owned())
            .collect()
    }
}

/// A scripted [`ProcessSource`] for tests: one frame of process names per poll.
///
/// Public rather than `#[cfg(test)]` so other modules' tests (the runtime's, in
/// particular) can drive a watcher without re-inventing this. Once the script
/// runs out every later poll reports an empty machine, which is the honest
/// answer — a test that polls past its own timeline should see the meetings
/// end, not silently repeat the last frame forever.
pub struct FakeProcessSource {
    frames: Vec<Vec<String>>,
    cursor: Mutex<usize>,
}

impl FakeProcessSource {
    pub fn new(frames: Vec<Vec<String>>) -> Self {
        FakeProcessSource {
            frames,
            cursor: Mutex::new(0),
        }
    }
}

impl ProcessSource for FakeProcessSource {
    fn running_processes(&self) -> Vec<String> {
        let mut cursor = self.cursor.lock().unwrap_or_else(|e| e.into_inner());
        let frame = self.frames.get(*cursor).cloned().unwrap_or_default();
        *cursor += 1;
        frame
    }
}

/// Debounce state for one known app.
#[derive(Debug, Default)]
struct AppState {
    /// Consecutive polls seen; reset to 0 the moment the app is missing.
    present_streak: u32,
    /// Consecutive polls missing; reset to 0 the moment the app is back.
    absent_streak: u32,
    /// A `Started` has fired and no `Ended` has followed. This flag, not the
    /// streaks, is what makes a meeting spanning a thousand polls fire once.
    running: bool,
}

/// Watches for meeting apps and reports debounced start/stop events.
///
/// Call [`poll`](Watcher::poll) on a timer; each call returns the events that
/// became true since the previous call, so the caller can hand them straight to
/// the UI without deduplicating.
pub struct Watcher {
    source: Box<dyn ProcessSource>,
    /// Parallel to [`KNOWN_APPS`], so iteration order — and therefore event
    /// order when two apps change in the same poll — is the table's order and
    /// not a hash map's whim.
    states: Vec<AppState>,
}

impl Watcher {
    pub fn new(source: Box<dyn ProcessSource>) -> Self {
        Watcher {
            source,
            states: KNOWN_APPS.iter().map(|_| AppState::default()).collect(),
        }
    }

    /// The production constructor: watch this machine's real processes.
    pub fn with_sysinfo() -> Self {
        Self::new(Box::new(SysinfoSource::new()))
    }

    /// One tick. Returns every meeting event that became true since the last
    /// call — usually none.
    ///
    /// The debounce state machine runs for every known app regardless of
    /// policy; only the *emission* is filtered by it. That is what keeps the
    /// events strictly alternating Started/Ended per app: a policy flipped to
    /// `Never` mid-meeting silences the `Ended` that matches an already-shown
    /// prompt, rather than desynchronising the two sides forever.
    pub fn poll(&mut self, settings: &Settings) -> Vec<MeetingEvent> {
        let present: HashSet<&'static str> = self
            .source
            .running_processes()
            .iter()
            .filter_map(|name| app_for_process(name))
            .map(|app| app.id)
            .collect();

        let mut events = Vec::new();
        for (app, state) in KNOWN_APPS.iter().zip(self.states.iter_mut()) {
            if present.contains(app.id) {
                state.present_streak = state.present_streak.saturating_add(1);
                state.absent_streak = 0;
            } else {
                state.absent_streak = state.absent_streak.saturating_add(1);
                state.present_streak = 0;
            }

            let kind = if !state.running && state.present_streak >= CONFIRM_POLLS {
                state.running = true;
                MeetingEventKind::Started
            } else if state.running && state.absent_streak >= CONFIRM_POLLS {
                state.running = false;
                MeetingEventKind::Ended
            } else {
                continue;
            };

            let policy = settings
                .auto_record
                .get(app.id)
                .copied()
                .unwrap_or_default();
            match policy {
                // "Never" is the user saying they never want to hear about this
                // app, so it suppresses the end of the meeting as well as the
                // start — an Ended for a prompt that was never shown is noise.
                AutoRecordPolicy::Never => continue,
                AutoRecordPolicy::Ask | AutoRecordPolicy::Always => {
                    events.push(MeetingEvent {
                        app_id: app.id.to_string(),
                        app_name: app.display_name.to_string(),
                        kind,
                        auto_start: policy == AutoRecordPolicy::Always,
                    });
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const N: usize = CONFIRM_POLLS as usize;

    /// One poll's worth of process names, repeated `count` times.
    fn frames(names: &[&str], count: usize) -> Vec<Vec<String>> {
        vec![names.iter().map(|s| s.to_string()).collect(); count]
    }

    fn watcher_over(script: Vec<Vec<String>>) -> Watcher {
        Watcher::new(Box::new(FakeProcessSource::new(script)))
    }

    fn settings_with(policies: &[(&str, AutoRecordPolicy)]) -> Settings {
        Settings {
            auto_record: policies
                .iter()
                .map(|(id, p)| (id.to_string(), *p))
                .collect::<BTreeMap<_, _>>(),
            ..Settings::default()
        }
    }

    /// Polls `count` times and returns every event, in order.
    fn poll_n(watcher: &mut Watcher, settings: &Settings, count: usize) -> Vec<MeetingEvent> {
        let mut all = Vec::new();
        for _ in 0..count {
            all.extend(watcher.poll(settings));
        }
        all
    }

    #[test]
    fn absent_present_absent_yields_exactly_one_started_then_one_ended() {
        let mut script = frames(&[], 2);
        script.extend(frames(&["zoom.us"], N));
        script.extend(frames(&[], N));
        let len = script.len();
        let mut watcher = watcher_over(script);
        let settings = Settings::default();

        let events = poll_n(&mut watcher, &settings, len);

        assert_eq!(events.len(), 2, "one Started and one Ended: {events:?}");
        assert_eq!(events[0].kind, MeetingEventKind::Started);
        assert_eq!(events[0].app_id, "zoom");
        assert_eq!(events[0].app_name, "Zoom");
        assert_eq!(events[1].kind, MeetingEventKind::Ended);
        assert_eq!(events[1].app_id, "zoom");
    }

    /// The debounce is the whole point: nothing fires until the app has been
    /// there for `CONFIRM_POLLS` polls in a row.
    #[test]
    fn started_waits_for_the_full_confirm_window() {
        let mut watcher = watcher_over(frames(&["zoom.us"], N));
        let settings = Settings::default();

        for poll in 1..N {
            assert!(
                watcher.poll(&settings).is_empty(),
                "fired early on poll {poll} of {N}"
            );
        }
        let events = watcher.poll(&settings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, MeetingEventKind::Started);
    }

    #[test]
    fn a_flapping_process_yields_no_events() {
        let mut script = Vec::new();
        for _ in 0..10 {
            script.extend(frames(&["zoom.us"], 1));
            script.extend(frames(&[], 1));
        }
        let len = script.len();
        let mut watcher = watcher_over(script);

        let events = poll_n(&mut watcher, &Settings::default(), len);
        assert!(events.is_empty(), "flapping must be silent: {events:?}");
    }

    #[test]
    fn a_continuous_meeting_fires_started_exactly_once() {
        let mut watcher = watcher_over(frames(&["zoom.us"], N + 50));
        let settings = Settings::default();

        let events = poll_n(&mut watcher, &settings, N + 50);
        assert_eq!(events.len(), 1, "one Started for one meeting: {events:?}");
        assert_eq!(events[0].kind, MeetingEventKind::Started);
    }

    #[test]
    fn policy_ask_is_the_default_and_does_not_auto_start() {
        let mut watcher = watcher_over(frames(&["zoom.us"], N));
        let events = poll_n(&mut watcher, &Settings::default(), N);

        assert_eq!(events.len(), 1);
        assert!(!events[0].auto_start, "Ask must prompt, never auto-start");
    }

    #[test]
    fn policy_always_sets_auto_start() {
        let mut script = frames(&["zoom.us"], N);
        script.extend(frames(&[], N));
        let len = script.len();
        let mut watcher = watcher_over(script);
        let settings = settings_with(&[("zoom", AutoRecordPolicy::Always)]);

        let events = poll_n(&mut watcher, &settings, len);
        assert_eq!(events.len(), 2);
        assert!(events[0].auto_start);
        assert_eq!(events[0].kind, MeetingEventKind::Started);
        assert_eq!(events[1].kind, MeetingEventKind::Ended);
    }

    #[test]
    fn policy_never_suppresses_started_and_ended_alike() {
        let mut script = frames(&["zoom.us"], N);
        script.extend(frames(&[], N));
        let len = script.len();
        let mut watcher = watcher_over(script);
        let settings = settings_with(&[("zoom", AutoRecordPolicy::Never)]);

        let events = poll_n(&mut watcher, &settings, len);
        assert!(events.is_empty(), "Never means silence: {events:?}");
    }

    #[test]
    fn two_meeting_apps_are_tracked_independently() {
        // Slack is up the whole time; Zoom joins late and leaves first.
        let mut script = frames(&["Slack"], N);
        script.extend(frames(&["Slack", "zoom.us"], N));
        script.extend(frames(&["Slack"], N));
        let len = script.len();
        let mut watcher = watcher_over(script);

        let events = poll_n(&mut watcher, &Settings::default(), len);
        let seen: Vec<(&str, MeetingEventKind)> =
            events.iter().map(|e| (e.app_id.as_str(), e.kind)).collect();
        assert_eq!(
            seen,
            vec![
                ("slack", MeetingEventKind::Started),
                ("zoom", MeetingEventKind::Started),
                ("zoom", MeetingEventKind::Ended),
            ],
            "slack must not end when zoom does"
        );
    }

    /// Per-app policy really is per app: silencing Slack must not silence
    /// Zoom.
    #[test]
    fn policies_apply_per_app() {
        let mut script = frames(&["Slack", "zoom.us"], N);
        script.extend(frames(&[], N));
        let len = script.len();
        let mut watcher = watcher_over(script);
        let settings = settings_with(&[
            ("slack", AutoRecordPolicy::Never),
            ("zoom", AutoRecordPolicy::Always),
        ]);

        let events = poll_n(&mut watcher, &settings, len);
        assert!(events.iter().all(|e| e.app_id == "zoom"), "{events:?}");
        assert_eq!(events.len(), 2);
        assert!(events[0].auto_start);
    }

    /// A browser being open is not a meeting — the watcher stays silent even
    /// when Chrome runs for the entire timeline.
    #[test]
    fn an_open_browser_never_produces_an_event() {
        let mut watcher = watcher_over(frames(&["Google Chrome", "chrome"], N * 3));
        let events = poll_n(&mut watcher, &Settings::default(), N * 3);
        assert!(events.is_empty(), "Chrome is not a meeting: {events:?}");
    }

    /// The fake proves the logic; this proves the production path. It asserts
    /// only what is true on any machine: enumeration returns something, and
    /// this very test binary is in the list.
    #[test]
    fn sysinfo_source_sees_this_process() {
        let source = SysinfoSource::new();
        let names = source.running_processes();
        assert!(!names.is_empty(), "process enumeration returned nothing");

        let exe = std::env::current_exe().expect("test binary path");
        let exe_name = exe
            .file_name()
            .expect("test binary file name")
            .to_string_lossy()
            .to_string();
        // Linux truncates a process name to 15 bytes, so match on prefix
        // rather than equality.
        assert!(
            names
                .iter()
                .any(|n| !n.is_empty() && exe_name.starts_with(n.as_str())),
            "this test binary ({exe_name}) was not in {names:?}"
        );

        // And the production wiring runs end to end. One poll can never reach
        // CONFIRM_POLLS, so "no events" is true on any machine — including one
        // that happens to have Slack open right now.
        let mut watcher = Watcher::with_sysinfo();
        assert!(watcher.poll(&Settings::default()).is_empty());
    }
}
