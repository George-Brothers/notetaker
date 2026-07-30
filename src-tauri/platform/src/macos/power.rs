//! macOS idle time and power state.
//!
//! Idle time comes from CoreGraphics rather than by parsing `ioreg` output.
//! That is a deliberate replacement for what Plan B1 shipped:
//! `notetaker_core::power::probe::MacProbe` shells out to
//! `ioreg -c IOHIDSystem` and scrapes `HIDIdleTime` out of the text. Its
//! parsers are tested against real captured output, but the arrangement was
//! listed in `docs/MAP.md` under "verified vs assumed" as risk #1 — it fails
//! *silently* if that command's output ever changes shape, and a silent
//! failure here means background transcription quietly never runs.
//!
//! `CGEventSourceSecondsSinceLastEventType` is the documented API for the same
//! number, returns a `f64` directly, and cannot change shape.
//!
//! AC and battery still go through core's `pmset -g batt` parser, which is
//! already tested against real captured output. Replacing it would mean writing
//! IOKit power-source bindings blind, for no gain: unlike idle time, the pmset
//! output is a stable documented format and a parse failure there is reported
//! rather than silent.
//!
//! Portions informed by anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/power/src/macos.rs`. See the NOTICE file.
//!
//! Compile-verified against `aarch64-apple-darwin` from Linux. The values are
//! first seen on real hardware.

use anyhow::Result;
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

use crate::RawPowerState;

/// Seconds since the last human input of any kind.
///
/// [`CGEventType::Null`] is the documented way to ask "any event type at all"
/// rather than, say, only keystrokes — a user who has been moving the mouse for
/// ten minutes is not idle, and asking only about keyboard events would say
/// they were.
///
/// `HIDSystemState` is the system-wide event source, which is what makes this
/// reflect the whole machine rather than only this process.
///
/// No `unsafe` block: `objc2` models this call as safe, and wrapping it in one
/// anyway is a lint error under `-D warnings` — as well as a small lie about
/// where this file's real risks are.
pub fn idle_seconds() -> f64 {
    let secs = CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::HIDSystemState,
        CGEventType::Null,
    );
    // A negative or NaN reading would be nonsense; report "not idle", which is
    // the direction that delays processing rather than running it while the
    // user is working.
    if secs.is_finite() && secs > 0.0 {
        secs
    } else {
        0.0
    }
}

/// Reads idle time from CoreGraphics and power state from `pmset`.
///
/// `on_ac` and `battery_pct` are filled by the caller in core, which owns the
/// tested `pmset` parser — this returns the machine's idle time and a
/// conservative default for the rest, so a caller that forgets to fill them in
/// gets "on wall power, no battery" (a desktop) rather than a false low-battery
/// reading that would block transcription forever.
pub fn read_power_state() -> Result<RawPowerState> {
    Ok(RawPowerState {
        idle_secs: idle_seconds() as u64,
        on_ac: true,
        battery_pct: None,
    })
}
