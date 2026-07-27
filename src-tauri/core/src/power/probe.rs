//! Reading real idle time and power state from the machine.
//!
//! macOS has no stable public API for "seconds since the user last touched
//! this machine" that we can reach from Rust without an Objective-C bridge,
//! so we shell out to the two tools that already report it: `ioreg` for
//! `HIDIdleTime` and `pmset` for the power source. Both are in the base
//! system, both have been stable for a decade, and neither needs entitlements.
//!
//! The split here is deliberate: the *shell-out* is `#[cfg(target_os =
//! "macos")]`, because those binaries only exist there, but the *parsing* is
//! cfg-free and public. That means the interesting half — everything that can
//! actually be wrong — is unit-tested on the Linux build box against captured
//! real output, and only the two-line `Command` invocation is unverifiable
//! until the code runs on the Mac.
//!
//! Every parser returns `Option`: a format we don't recognise is `None`, never
//! a guess. [`crate::power::PowerPolicy`] turns that `None` into "not idle",
//! so a broken probe delays processing rather than running it while the user
//! is mid-sentence on battery.

use super::{PowerState, SystemProbe};

/// Seconds since the last HID (keyboard/mouse/trackpad) event, parsed out of
/// `ioreg -c IOHIDSystem`. The registry reports nanoseconds:
///
/// ```text
///   "HIDIdleTime" = 1339059157000
/// ```
///
/// Only the first occurrence is read — `-c IOHIDSystem` yields one node.
pub fn parse_idle_secs(ioreg_output: &str) -> Option<u64> {
    let line = ioreg_output
        .lines()
        .find(|l| l.contains("\"HIDIdleTime\""))?;
    let nanos: u64 = line.split('=').nth(1)?.trim().parse().ok()?;
    Some(nanos / 1_000_000_000)
}

/// `(on_ac, battery_pct)` parsed out of `pmset -g batt`:
///
/// ```text
/// Now drawing from 'AC Power'
///  -InternalBattery-0 (id=4653155)    87%; charged; 0:00 remaining present: true
/// ```
///
/// (the gap before `87%` is a literal tab in the real output; the test
/// fixtures keep it)
///
/// The battery line is absent on a desktop, which is a legitimate reading
/// (`battery_pct: None`), not a parse failure. A missing *power source* line
/// is a parse failure, because that is the half we gate on.
pub fn parse_power(pmset_output: &str) -> Option<(bool, Option<u8>)> {
    let source = pmset_output
        .lines()
        .find(|l| l.contains("Now drawing from"))?;
    let on_ac = if source.contains("'AC Power'") {
        true
    } else if source.contains("'Battery Power'") {
        false
    } else {
        // Some third power-source wording we have never seen: refuse to guess.
        return None;
    };

    // Battery lines start with a dash and carry the charge as `NN%`.
    let battery_pct = pmset_output
        .lines()
        .find(|l| l.trim_start().starts_with('-') && l.contains('%'))
        .and_then(parse_percent);

    Some((on_ac, battery_pct))
}

/// The `NN` in the first `NN%` on a line, rejecting anything over 100 so a
/// garbled reading degrades to "no battery info" rather than a bogus number.
fn parse_percent(line: &str) -> Option<u8> {
    let end = line.find('%')?;
    let start = line.as_bytes()[..end]
        .iter()
        .rposition(|b| !b.is_ascii_digit())
        .map_or(0, |i| i + 1);
    // `get` rather than indexing: a non-char-boundary slice yields `None`
    // instead of panicking on unexpected input.
    let pct: u8 = line.get(start..end)?.parse().ok()?;
    (pct <= 100).then_some(pct)
}

/// Assembles a [`PowerState`] from the raw output of both commands. Kept
/// cfg-free so the whole read path except the two `Command` calls is testable
/// off a Mac.
pub fn parse_state(ioreg_output: &str, pmset_output: &str) -> Option<PowerState> {
    let idle_secs = parse_idle_secs(ioreg_output)?;
    let (on_ac, battery_pct) = parse_power(pmset_output)?;
    Some(PowerState {
        idle_secs,
        on_ac,
        battery_pct,
    })
}

/// The real macOS probe. UNVERIFIED off a Mac: the parsers below it are
/// covered by tests, but this shell-out itself is first exercised on the
/// hardware.
#[cfg(target_os = "macos")]
pub struct MacProbe;

#[cfg(target_os = "macos")]
impl SystemProbe for MacProbe {
    fn read(&self) -> Option<PowerState> {
        let ioreg = capture("ioreg", &["-c", "IOHIDSystem"])?;
        let pmset = capture("pmset", &["-g", "batt"])?;
        parse_state(&ioreg, &pmset)
    }
}

/// Runs a command and returns its stdout, or `None` on any failure to launch,
/// a non-zero exit, or non-UTF-8 output.
#[cfg(target_os = "macos")]
fn capture(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// A probe that reports a fixed reading. Used by this crate's tests and as the
/// stand-in on platforms with no real probe — on Linux there is no idle-time
/// API to read, so the runtime supplies a `FakeProbe` (or keeps
/// `queue::AlwaysIdle`) rather than pretending to measure something.
pub struct FakeProbe {
    pub state: Option<PowerState>,
}

impl SystemProbe for FakeProbe {
    fn read(&self) -> Option<PowerState> {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `ioreg -c IOHIDSystem` output, trimmed to the relevant node.
    const IOREG_IDLE: &str = r#"+-o Root  <class IORegistryEntry, id 0x100000100, retain 38>
  +-o J316sAP  <class IOPlatformExpertDevice, id 0x100000267, registered, matched, active, busy 0 (7 ms), retain 41>
    +-o AppleARMPE  <class AppleARMPE, id 0x10000026c, registered, matched, active, busy 0 (0 ms), retain 8>
      +-o IOHIDSystem  <class IOHIDSystem, id 0x1000004ff, registered, matched, active, busy 0 (0 ms), retain 6>
        {
          "IOClass" = "IOHIDSystem"
          "HIDIdleTime" = 1339059157000
          "IOProviderClass" = "IOResources"
          "HIDPointerAcceleration" = 45056
        }
"#;

    const PMSET_AC_CHARGED: &str = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=4653155)\t87%; charged; 0:00 remaining present: true\n";

    const PMSET_BATTERY_DISCHARGING: &str = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t42%; discharging; 3:21 remaining present: true\n";

    const PMSET_DESKTOP: &str = "Now drawing from 'AC Power'\n";

    #[test]
    fn parses_hid_idle_time_nanoseconds_into_seconds() {
        assert_eq!(parse_idle_secs(IOREG_IDLE), Some(1339));
    }

    #[test]
    fn ioreg_without_hid_idle_time_is_none() {
        assert_eq!(parse_idle_secs("+-o Root  <class IORegistryEntry>\n"), None);
    }

    #[test]
    fn ioreg_with_non_numeric_idle_time_is_none() {
        assert_eq!(
            parse_idle_secs("          \"HIDIdleTime\" = <not a number>\n"),
            None
        );
        assert_eq!(parse_idle_secs("          \"HIDIdleTime\"\n"), None);
    }

    #[test]
    fn parses_pmset_on_ac_with_charged_battery() {
        assert_eq!(parse_power(PMSET_AC_CHARGED), Some((true, Some(87))));
    }

    #[test]
    fn parses_pmset_on_battery_while_discharging() {
        assert_eq!(
            parse_power(PMSET_BATTERY_DISCHARGING),
            Some((false, Some(42)))
        );
    }

    #[test]
    fn parses_pmset_on_a_desktop_as_ac_with_no_battery() {
        assert_eq!(parse_power(PMSET_DESKTOP), Some((true, None)));
    }

    #[test]
    fn pmset_without_a_power_source_line_is_none() {
        assert_eq!(parse_power(""), None);
        assert_eq!(parse_power("pmset: command not found\n"), None);
    }

    #[test]
    fn pmset_with_an_impossible_percentage_reports_no_battery() {
        let out = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t420%; discharging; 3:21 remaining\n";
        assert_eq!(parse_power(out), Some((false, None)));
    }

    #[test]
    fn parse_state_combines_both_command_outputs() {
        assert_eq!(
            parse_state(IOREG_IDLE, PMSET_BATTERY_DISCHARGING),
            Some(crate::power::PowerState {
                idle_secs: 1339,
                on_ac: false,
                battery_pct: Some(42),
            })
        );
    }

    #[test]
    fn parse_state_is_none_when_either_output_is_malformed() {
        assert_eq!(parse_state("garbage", PMSET_AC_CHARGED), None);
        assert_eq!(parse_state(IOREG_IDLE, "garbage"), None);
    }

    #[test]
    fn fake_probe_returns_whatever_it_was_given() {
        use crate::power::SystemProbe;
        let state = crate::power::PowerState {
            idle_secs: 10,
            on_ac: true,
            battery_pct: None,
        };
        assert_eq!(FakeProbe { state: Some(state) }.read(), Some(state));
        assert_eq!(FakeProbe { state: None }.read(), None);
    }
}
