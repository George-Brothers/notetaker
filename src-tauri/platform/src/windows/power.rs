//! Windows idle time and power state.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/power/src/windows.rs`. See the NOTICE file. Extended here
//! with idle time, which their version does not read.
//!
//! Compile-verified against `x86_64-pc-windows-msvc` from Linux; the values
//! these calls return are first seen on a real machine or in CI.

use anyhow::{Context, Result};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

use crate::RawPowerState;

/// `GetSystemPowerStatus` reports this when it cannot tell whether the machine
/// is on wall power.
const AC_LINE_UNKNOWN: u8 = 255;
/// ...and this for a battery level it cannot read.
const BATTERY_PCT_UNKNOWN: u8 = 255;
/// `ACLineStatus` value meaning "on wall power".
const AC_LINE_ONLINE: u8 = 1;
/// `BatteryFlag` bit meaning "no system battery" — a desktop.
const BATTERY_FLAG_NO_BATTERY: u8 = 128;

/// Reads the machine's idle time and power state.
///
/// Returns `Err` rather than a guess when Windows will not answer. The caller
/// in core treats an unreadable machine as "not idle", so a failure here
/// delays background transcription instead of running it at a bad moment.
pub fn read_power_state() -> Result<RawPowerState> {
    Ok(RawPowerState {
        idle_secs: idle_seconds()?,
        on_ac: on_ac_power()?,
        battery_pct: battery_percent()?,
    })
}

/// Seconds since the last keyboard or mouse event.
///
/// `GetLastInputInfo` gives the tick count at the last input, and
/// `GetTickCount` the current one; the difference is the idle time.
/// `wrapping_sub` is not incidental — `GetTickCount` is a 32-bit millisecond
/// counter that rolls over every 49.7 days, and on the wrong side of a rollover
/// a plain subtraction underflows to roughly 49 days of apparent idleness.
/// A machine that has been up seven weeks would suddenly look permanently idle
/// and start transcribing under the user's hands.
pub fn idle_seconds() -> Result<u64> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a correctly-sized, initialized LASTINPUTINFO, as the
    // API requires — `cbSize` is set above and Windows only writes `dwTime`.
    let ok = unsafe { GetLastInputInfo(&mut info) };
    anyhow::ensure!(ok.as_bool(), "GetLastInputInfo failed");
    // SAFETY: no arguments, no outputs, always succeeds.
    let now = unsafe { GetTickCount() };
    Ok((now.wrapping_sub(info.dwTime) / 1_000) as u64)
}

/// True when running on wall power.
///
/// An *unknown* AC status is reported as `true` — on wall power. This is the
/// deliberate direction: the only thing `on_ac` gates is whether background
/// transcription may run, and the machines that cannot answer are typically
/// desktops and VMs with no battery at all. Reporting `false` there would mean
/// a desktop user with `require_ac` on never gets a transcript, and would look
/// like the app is simply broken.
pub fn on_ac_power() -> Result<bool> {
    let status = power_status()?;
    if status.ACLineStatus == AC_LINE_UNKNOWN {
        return Ok(true);
    }
    Ok(status.ACLineStatus == AC_LINE_ONLINE)
}

/// Battery charge 0..=100, or `None` on a machine with no battery.
pub fn battery_percent() -> Result<Option<u8>> {
    let status = power_status()?;
    if status.BatteryFlag & BATTERY_FLAG_NO_BATTERY != 0
        || status.BatteryLifePercent == BATTERY_PCT_UNKNOWN
    {
        return Ok(None);
    }
    // Clamped rather than trusted: core's battery floor compares against 100
    // and a value above it would be nonsense to show a user.
    Ok(Some(status.BatteryLifePercent.min(100)))
}

fn power_status() -> Result<SYSTEM_POWER_STATUS> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a valid, zeroed SYSTEM_POWER_STATUS which is all the
    // API needs; it only writes into it.
    unsafe { GetSystemPowerStatus(&mut status) }.context("GetSystemPowerStatus failed")?;
    Ok(status)
}
