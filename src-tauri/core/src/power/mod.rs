//! Idle and power gating: deciding when it is polite to spend the machine's
//! CPU on transcription. Mr. Brothers' choice was "when the Mac is idle and
//! plugged in", so that is exactly what this encodes.
//!
//! Plan A shipped `queue::AlwaysIdle` as a placeholder. This module provides
//! the real [`PowerPolicy`], which implements the same `IdleSource` trait, so
//! swapping it in is a one-line change in the runtime.
//!
//! The *decision* is pure and fully tested here; only the [`SystemProbe`] that
//! reads the machine's actual idle time and power state is platform-bound.

pub mod probe;

use serde::{Deserialize, Serialize};

/// A reading of the machine's current idle and power state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerState {
    /// Seconds since the last keyboard/mouse event.
    pub idle_secs: u64,
    /// True when running on wall power rather than battery.
    pub on_ac: bool,
    /// Battery percentage 0..=100, or `None` on a machine without a battery
    /// (a desktop is never "low battery").
    pub battery_pct: Option<u8>,
}

/// Reads [`PowerState`] from the machine. Implemented by a real macOS probe
/// (`ioreg`/`pmset`) and by a fake in tests.
///
/// A probe that cannot read the machine returns `None` rather than guessing —
/// the policy treats an unknown machine as "not idle", so a broken probe
/// delays processing instead of running it at a bad moment.
pub trait SystemProbe {
    fn read(&self) -> Option<PowerState>;
}

use crate::api::Settings;
use crate::queue::IdleSource;

/// Battery floor for background processing, in percent.
///
/// Transcription is a sustained multi-minute CPU burn. Below this the cost of
/// draining the last of the battery — the user closes the lid and finds a dead
/// Mac — outweighs finishing a recording that is in no hurry. Only applies
/// while actually on battery; a machine on wall power at 5% is charging.
pub const MIN_BATTERY_PCT: u8 = 30;

/// The whole gating decision, as a pure function over a machine reading and
/// the user's settings, so every combination can be table-tested.
///
/// `process_when_idle == false` means **"run whenever"**, not "never run" —
/// it is the user turning the idle *restriction* off. Reading it the other way
/// would mean the app quietly transcribes nothing, so it is the first thing
/// checked and the first thing tested.
pub fn should_run(state: PowerState, settings: &Settings) -> bool {
    if !settings.process_when_idle {
        return true;
    }
    let idle_enough = state.idle_secs >= settings.min_idle_secs;
    let power_ok = !settings.require_ac || state.on_ac;
    idle_enough && power_ok && battery_ok(state)
}

/// True unless we are on battery and below [`MIN_BATTERY_PCT`]. A machine
/// reporting no battery at all (a desktop) can never be "low battery", and the
/// floor is irrelevant on wall power.
fn battery_ok(state: PowerState) -> bool {
    if state.on_ac {
        return true;
    }
    match state.battery_pct {
        Some(pct) => pct >= MIN_BATTERY_PCT,
        None => true,
    }
}

/// The real [`IdleSource`] for the processing queue: reads the machine through
/// a [`SystemProbe`], then applies [`should_run`]. Replaces Plan A's
/// `queue::AlwaysIdle` in production.
///
/// Holds a snapshot of [`Settings`] rather than a live handle. The runtime
/// rebuilds the policy when settings change, which keeps this type free of
/// locks and keeps `ok_to_run` non-blocking — the scheduler calls it on every
/// tick.
pub struct PowerPolicy<P: SystemProbe> {
    probe: P,
    settings: Settings,
}

impl<P: SystemProbe> PowerPolicy<P> {
    pub fn new(probe: P, settings: Settings) -> Self {
        Self { probe, settings }
    }
}

impl<P: SystemProbe + Send + Sync> IdleSource for PowerPolicy<P> {
    fn ok_to_run(&self) -> bool {
        // Short-circuit before touching the probe: "run whenever" has no
        // question to ask the machine, and on macOS `read()` is two
        // process spawns we would otherwise pay for on every tick. This also
        // means a probe that cannot read the machine never blocks a user who
        // explicitly asked for unrestricted processing.
        if !self.settings.process_when_idle {
            return true;
        }
        match self.probe.read() {
            Some(state) => should_run(state, &self.settings),
            // Unknown machine: treat as busy. A broken probe must delay
            // processing, never start it at a bad moment.
            None => false,
        }
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use crate::api::Settings;
    use crate::power::probe::FakeProbe;
    use crate::queue::IdleSource;

    /// Settings with an explicit idle threshold, so the boundary cases below
    /// are readable rather than depending on whatever the default happens to
    /// be.
    fn settings(process_when_idle: bool, require_ac: bool) -> Settings {
        Settings {
            process_when_idle,
            min_idle_secs: 300,
            require_ac,
            ..Settings::default()
        }
    }

    struct Case {
        name: &'static str,
        idle_secs: u64,
        on_ac: bool,
        battery_pct: Option<u8>,
        process_when_idle: bool,
        require_ac: bool,
        expect: bool,
    }

    #[test]
    fn decision_table_over_idle_ac_and_battery() {
        let cases = [
            Case {
                name: "idle, plugged in, healthy battery",
                idle_secs: 900,
                on_ac: true,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: true,
                expect: true,
            },
            Case {
                name: "idle exactly at the threshold is allowed",
                idle_secs: 300,
                on_ac: true,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: true,
                expect: true,
            },
            Case {
                name: "one second under the threshold is not",
                idle_secs: 299,
                on_ac: true,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: true,
                expect: false,
            },
            Case {
                name: "user actively typing",
                idle_secs: 0,
                on_ac: true,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: true,
                expect: false,
            },
            Case {
                name: "on battery while require_ac is on",
                idle_secs: 900,
                on_ac: false,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: true,
                expect: false,
            },
            Case {
                name: "on battery is fine once require_ac is off",
                idle_secs: 900,
                on_ac: false,
                battery_pct: Some(90),
                process_when_idle: true,
                require_ac: false,
                expect: true,
            },
            Case {
                name: "battery exactly at the floor is allowed",
                idle_secs: 900,
                on_ac: false,
                battery_pct: Some(MIN_BATTERY_PCT),
                process_when_idle: true,
                require_ac: false,
                expect: true,
            },
            Case {
                name: "one point under the floor is not",
                idle_secs: 900,
                on_ac: false,
                battery_pct: Some(MIN_BATTERY_PCT - 1),
                process_when_idle: true,
                require_ac: false,
                expect: false,
            },
            Case {
                name: "nearly flat battery",
                idle_secs: 900,
                on_ac: false,
                battery_pct: Some(3),
                process_when_idle: true,
                require_ac: false,
                expect: false,
            },
            Case {
                name: "the floor does not apply while charging",
                idle_secs: 900,
                on_ac: true,
                battery_pct: Some(5),
                process_when_idle: true,
                require_ac: true,
                expect: true,
            },
            Case {
                name: "a desktop has no battery to be low on",
                idle_secs: 900,
                on_ac: true,
                battery_pct: None,
                process_when_idle: true,
                require_ac: true,
                expect: true,
            },
            Case {
                name: "no battery reading is never treated as low battery",
                idle_secs: 900,
                on_ac: false,
                battery_pct: None,
                process_when_idle: true,
                require_ac: false,
                expect: true,
            },
            Case {
                name: "not idle and flat battery still fails",
                idle_secs: 0,
                on_ac: false,
                battery_pct: Some(2),
                process_when_idle: true,
                require_ac: false,
                expect: false,
            },
            Case {
                name: "run whenever: busy machine, on battery, nearly flat",
                idle_secs: 0,
                on_ac: false,
                battery_pct: Some(3),
                process_when_idle: false,
                require_ac: true,
                expect: true,
            },
            Case {
                name: "run whenever: an otherwise-good machine still runs",
                idle_secs: 900,
                on_ac: true,
                battery_pct: Some(90),
                process_when_idle: false,
                require_ac: true,
                expect: true,
            },
        ];

        for case in cases {
            let state = PowerState {
                idle_secs: case.idle_secs,
                on_ac: case.on_ac,
                battery_pct: case.battery_pct,
            };
            let settings = settings(case.process_when_idle, case.require_ac);
            assert_eq!(
                should_run(state, &settings),
                case.expect,
                "case: {}",
                case.name
            );
        }
    }

    #[test]
    fn a_probe_that_cannot_read_the_machine_is_not_idle() {
        let policy = PowerPolicy::new(FakeProbe { state: None }, settings(true, true));
        assert!(
            !policy.ok_to_run(),
            "a broken probe must delay processing, never run it at a bad moment"
        );
    }

    #[test]
    fn process_when_idle_off_means_run_whenever_not_never() {
        // The expensive mistake: reading `process_when_idle == false` as
        // "never process" would mean the app silently transcribes nothing.
        let policy = PowerPolicy::new(
            FakeProbe {
                state: Some(PowerState {
                    idle_secs: 0,
                    on_ac: false,
                    battery_pct: Some(3),
                }),
            },
            settings(false, true),
        );
        assert!(policy.ok_to_run());
    }

    #[test]
    fn run_whenever_survives_a_broken_probe_too() {
        // "Run whenever" does not consult the machine at all, so a probe that
        // cannot read it is irrelevant.
        let policy = PowerPolicy::new(FakeProbe { state: None }, settings(false, true));
        assert!(policy.ok_to_run());
    }

    #[test]
    fn policy_gates_on_the_probe_reading() {
        let good = PowerState {
            idle_secs: 900,
            on_ac: true,
            battery_pct: Some(90),
        };
        let busy = PowerState {
            idle_secs: 5,
            ..good
        };
        assert!(
            PowerPolicy::new(FakeProbe { state: Some(good) }, settings(true, true)).ok_to_run()
        );
        assert!(
            !PowerPolicy::new(FakeProbe { state: Some(busy) }, settings(true, true)).ok_to_run()
        );
    }
}
