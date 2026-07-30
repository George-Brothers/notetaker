//! The table of known meeting apps and how to recognize them.
//!
//! Detection is by process name because that is the only signal available
//! without extra macOS permissions: enumerating processes needs nothing, while
//! reading window titles needs Accessibility and inspecting audio devices needs
//! a running capture. A first-run experience that asks for two more permissions
//! before it can offer to record is worse than a table of names.
//!
//! **What process presence actually proves.** It proves the app is *open*, not
//! that a call is happening. That is a usable signal here only because the
//! watcher emits *transitions*, not states: an app that launches at login fires
//! one prompt for that login session, and "Never for Slack" silences it
//! forever. Browsers are the case where that argument collapses — see
//! [`BROWSER_APPS`].
//!
//! **Every shipping platform's process names are listed together**, in one
//! table, matched case-insensitively. macOS and Windows are both targets;
//! Linux names are here too because the fake-driven tests and all dev-machine
//! runs happen on Linux. Linux truncates `comm` to 15 bytes, so a longer Linux
//! name may not match there — harmless, as Linux is not a shipping platform.
//!
//! Windows names carry the `.exe` suffix because that is what `sysinfo` reports
//! there, and [`app_for_process`] compares whole names rather than substrings.
//! One table rather than a `cfg` per platform: a name in the wrong arm of a
//! `cfg` cannot be tested from the other OS, and this table is the part most
//! likely to need editing by someone who only has one of the two machines.

/// One known meeting app: the id policy is stored under, the name a human
/// sees, and every process name that counts as "this app is running".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingApp {
    /// Stable key for `api::Settings::auto_record`. Never change one of these
    /// without a migration — a renamed id silently resets a user's policy.
    pub id: &'static str,
    /// What the prompt says: "Zoom started — record this?"
    pub display_name: &'static str,
    /// Matched case-insensitively against `sysinfo`'s process names.
    pub process_names: &'static [&'static str],
}

/// The apps the watcher actually detects.
pub const KNOWN_APPS: &[MeetingApp] = &[
    MeetingApp {
        id: "zoom",
        display_name: "Zoom",
        // macOS names the binary after the bundle: `zoom.us`. Linux ships
        // `zoom`. Windows ships `Zoom.exe`. `ZoomPhone` is deliberately absent
        // — a phone call is not a meeting we have both sides of.
        //
        // `CptHost.exe` is Zoom's Windows meeting-host process, which exists
        // only while a meeting is actually running. That makes it a genuinely
        // better signal than "the app is open" — the one place on either
        // platform where process presence really does mean "in a call".
        process_names: &["zoom.us", "zoom", "Zoom.exe", "CptHost.exe"],
    },
    MeetingApp {
        id: "teams",
        display_name: "Microsoft Teams",
        // `MSTeams` is Teams 2.x (2023+); `Teams` is the classic Electron
        // build, still deployed in plenty of orgs. Linux: `teams-for-linux`,
        // the community client — exactly 15 bytes, so it survives truncation.
        // Windows: `ms-teams.exe` is the new client, `Teams.exe` the classic.
        process_names: &[
            "MSTeams",
            "Teams",
            "teams-for-linux",
            "ms-teams.exe",
            "Teams.exe",
        ],
    },
    MeetingApp {
        id: "slack",
        display_name: "Slack",
        // Slack huddles. Slack is also open all day for most people, so this
        // leans hard on "one prompt per launch, then Never for Slack".
        process_names: &["Slack", "slack.exe"],
    },
    MeetingApp {
        id: "webex",
        display_name: "Webex",
        // `Webex` is the unified Webex App. `Cisco Webex Meetings` is the
        // standalone meetings client some enterprises still pin to. On Windows
        // the unified app's main process is `CiscoCollabHost.exe`, which is not
        // guessable from the macOS name — hence listing it explicitly.
        process_names: &[
            "Webex",
            "Cisco Webex Meetings",
            "CiscoCollabHost.exe",
            "Webex.exe",
        ],
    },
    MeetingApp {
        id: "discord",
        display_name: "Discord",
        // Capitalized on macOS and the Linux AppImage alike; `Discord.exe` on
        // Windows.
        process_names: &["Discord", "Discord.exe"],
    },
    MeetingApp {
        id: "facetime",
        display_name: "FaceTime",
        // macOS only; there is no Linux equivalent, so this entry can only
        // ever match in production.
        process_names: &["FaceTime"],
    },
];

/// Browsers, recorded but **not detected** — the honest answer to "how do we
/// see Google Meet?"
///
/// Chrome running does not mean a Meet call is happening; it means a browser is
/// open. Unlike Slack or Zoom, a browser is launched and quit repeatedly
/// through a normal day, so wiring these into [`app_for_process`] would fire a
/// "record this?" prompt several times a day for reasons that have nothing to
/// do with meetings — the fastest possible way to teach someone to dismiss the
/// prompt without reading it. A detector that cries wolf is worse than no
/// detector.
///
/// The table stays so the research is not lost: on the Mac (B2), Meet detection
/// gets a real signal — the window title (`"Meet - …"`) or the fact that the
/// browser holds an active microphone stream — and this entry is what it will
/// be keyed to. Until then nothing here reaches the watcher, which
/// `watcher::tests::an_open_browser_never_produces_an_event` pins down.
pub const BROWSER_APPS: &[MeetingApp] = &[MeetingApp {
    id: "meet",
    display_name: "Google Meet",
    // macOS: `Google Chrome`, `Chromium`, `Microsoft Edge`. Linux: `chrome`,
    // `chromium`, `chromium-browser`, `microsoft-edge`.
    process_names: &[
        "Google Chrome",
        "chrome",
        "Chromium",
        "chromium-browser",
        "Microsoft Edge",
        "microsoft-edge",
    ],
}];

/// The app a running process belongs to, or `None` for everything else —
/// including browsers, per [`BROWSER_APPS`].
///
/// Case-insensitive because macOS says `Slack` and Linux says `slack`, and this
/// code is developed on one and shipped on the other.
pub fn app_for_process(process_name: &str) -> Option<&'static MeetingApp> {
    if process_name.is_empty() {
        return None;
    }
    KNOWN_APPS.iter().find(|app| {
        app.process_names
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(process_name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every meeting app that exists on Windows must have a Windows process
    /// name, or detection silently does nothing there — the app would simply
    /// never offer to record, with no error to notice.
    ///
    /// FaceTime is exempt: it is macOS-only and has no Windows build.
    #[test]
    fn every_cross_platform_app_has_a_windows_process_name() {
        for app in KNOWN_APPS {
            if app.id == "facetime" {
                continue;
            }
            assert!(
                app.process_names
                    .iter()
                    .any(|n| n.to_ascii_lowercase().ends_with(".exe")),
                "{} has no Windows (.exe) process name, so it can never be \
                 detected on a PC",
                app.id
            );
        }
    }

    /// ...and the mirror: each must still be detectable on a Mac.
    #[test]
    fn every_app_still_has_a_non_windows_process_name() {
        for app in KNOWN_APPS {
            assert!(
                app.process_names
                    .iter()
                    .any(|n| !n.to_ascii_lowercase().ends_with(".exe")),
                "{} only has a Windows process name",
                app.id
            );
        }
    }

    /// The real names `sysinfo` reports on Windows, resolved to the right app.
    #[test]
    fn windows_process_names_resolve_to_the_right_app() {
        for (process, expected_id) in [
            ("Zoom.exe", "zoom"),
            ("CptHost.exe", "zoom"),
            ("ms-teams.exe", "teams"),
            ("Teams.exe", "teams"),
            ("slack.exe", "slack"),
            ("Discord.exe", "discord"),
            ("CiscoCollabHost.exe", "webex"),
        ] {
            let got = app_for_process(process)
                .unwrap_or_else(|| panic!("{process} was not detected at all"));
            assert_eq!(got.id, expected_id, "{process} resolved to the wrong app");
        }
    }

    /// Windows process names arrive with inconsistent casing depending on how
    /// the app was installed and launched, so matching must not depend on it.
    #[test]
    fn windows_names_match_regardless_of_case() {
        assert_eq!(app_for_process("ZOOM.EXE").map(|a| a.id), Some("zoom"));
        assert_eq!(app_for_process("slack.EXE").map(|a| a.id), Some("slack"));
        assert_eq!(
            app_for_process("MS-TEAMS.EXE").map(|a| a.id),
            Some("teams")
        );
    }

    /// No process name may be claimed by two apps — an overlap would make
    /// detection order-dependent, and `auto_record` policy would attach to
    /// whichever app happened to be listed first.
    #[test]
    fn no_process_name_is_claimed_by_two_apps() {
        let mut seen: Vec<(String, &str)> = Vec::new();
        for app in KNOWN_APPS {
            for name in app.process_names {
                let lower = name.to_ascii_lowercase();
                if let Some((_, other)) = seen.iter().find(|(n, _)| *n == lower) {
                    panic!("process name {name:?} is claimed by both {other} and {}", app.id);
                }
                seen.push((lower, app.id));
            }
        }
    }

    /// A `.exe` we do not know about must not match anything. Guards against a
    /// future change to substring matching, which would make `Zoom.exe` match
    /// `NotZoom.exe`.
    #[test]
    fn an_unknown_windows_process_matches_nothing() {
        assert!(app_for_process("explorer.exe").is_none());
        assert!(app_for_process("NotZoom.exe").is_none());
        assert!(app_for_process("Zoom.exe.bak").is_none());
    }

    #[test]
    fn covers_the_apps_the_plan_requires() {
        let ids: Vec<&str> = KNOWN_APPS.iter().map(|a| a.id).collect();
        for required in ["zoom", "teams", "slack", "webex", "discord", "facetime"] {
            assert!(ids.contains(&required), "missing known app {required}");
        }
    }

    #[test]
    fn looks_up_the_real_macos_process_names() {
        assert_eq!(app_for_process("zoom.us").map(|a| a.id), Some("zoom"));
        assert_eq!(app_for_process("MSTeams").map(|a| a.id), Some("teams"));
        assert_eq!(app_for_process("Slack").map(|a| a.id), Some("slack"));
        assert_eq!(app_for_process("FaceTime").map(|a| a.id), Some("facetime"));
        assert_eq!(
            app_for_process("zoom.us").map(|a| a.display_name),
            Some("Zoom")
        );
    }

    /// Linux and macOS disagree on capitalization (`slack` vs `Slack`), and
    /// the fake-driven tests run on Linux, so matching cannot be case
    /// sensitive.
    #[test]
    fn lookup_is_case_insensitive_across_platforms() {
        assert_eq!(app_for_process("slack").map(|a| a.id), Some("slack"));
        assert_eq!(app_for_process("SLACK").map(|a| a.id), Some("slack"));
        assert_eq!(app_for_process("zoom").map(|a| a.id), Some("zoom"));
    }

    #[test]
    fn unknown_processes_match_nothing() {
        assert!(app_for_process("bash").is_none());
        assert!(app_for_process("").is_none());
    }

    /// The honest half of Meet detection: a live browser process is not a
    /// meeting. Chrome must never resolve to an app through the detection
    /// path.
    #[test]
    fn browsers_are_not_detected_as_meetings() {
        for browser in ["Google Chrome", "chrome", "chromium", "firefox", "Safari"] {
            assert!(
                app_for_process(browser).is_none(),
                "{browser} must not be treated as a meeting"
            );
        }
    }

    /// The Meet entry still exists, parked in `BROWSER_APPS`, so B2 can wire
    /// it to a real signal without redoing the process-name research.
    #[test]
    fn meet_is_parked_in_the_browser_table() {
        let meet = BROWSER_APPS
            .iter()
            .find(|a| a.id == "meet")
            .expect("Meet entry recorded for the Mac-side follow-up");
        assert_eq!(meet.display_name, "Google Meet");
        assert!(meet
            .process_names
            .iter()
            .any(|p| p.eq_ignore_ascii_case("Google Chrome")));
    }

    #[test]
    fn ids_and_process_names_are_unambiguous() {
        let mut seen_ids = Vec::new();
        let mut seen_procs = Vec::new();
        for app in KNOWN_APPS.iter().chain(BROWSER_APPS) {
            assert!(!seen_ids.contains(&app.id), "duplicate app id {}", app.id);
            seen_ids.push(app.id);
            for p in app.process_names {
                let lower = p.to_lowercase();
                assert!(
                    !seen_procs.contains(&lower),
                    "process name {p} claimed by two apps"
                );
                seen_procs.push(lower);
            }
        }
    }
}
