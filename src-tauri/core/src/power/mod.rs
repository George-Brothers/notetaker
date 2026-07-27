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
