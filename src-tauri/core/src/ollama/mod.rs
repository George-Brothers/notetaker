//! Managing the local LLM: is Ollama installed and running, is the model we
//! need present, and pulling it with real progress.
//!
//! Summarization is the one part of the pipeline that depends on software the
//! app does not ship. Ollama is a separate download the user installs by
//! clicking a button on a web page, so every failure here is something the
//! user has to *do* something about — which means none of them may surface as
//! a stack trace. [`status`] answers "what is wrong right now" as a plain
//! value (never an `Err`, not even when the port is dead), and carries the
//! sentence the UI shows when Ollama is missing entirely.
//!
//! [`pull`] streams Ollama's newline-delimited JSON progress feed and reports
//! a percent as the bytes land, so the first-run screen shows a real bar for
//! what is a multi-gigabyte download. Progress goes through a `FnMut`
//! callback in the same style as [`crate::models::Downloader::ensure`], so
//! the UI has one mental model for "something large is downloading" rather
//! than two.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Whole-request budget for [`status`]. This runs on the UI's critical path
/// (first-run checklist, settings screen), so a wedged server must not hang
/// the window — a few seconds is generous for a localhost `GET`.
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// Connect-only cap for [`pull`]. Deliberately *not* a global timeout: a real
/// pull moves gigabytes and legitimately runs for many minutes, so only the
/// "is anything listening" phase is bounded.
const PULL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The executable's name, looked up on `PATH` to tell "installed but not
/// running" apart from "not installed".
#[cfg(windows)]
const BINARY_NAME: &str = "ollama.exe";
#[cfg(not(windows))]
const BINARY_NAME: &str = "ollama";

/// System-wide Windows installer location, which is not necessarily on `PATH`.
#[cfg(windows)]
const EXTRA_INSTALL_LOCATIONS: &[&str] = &[r"C:\Program Files\Ollama\ollama.exe"];

/// Places Ollama lands that are not necessarily on `PATH`. The macOS app
/// bundle keeps its CLI inside itself and only symlinks it into
/// `/usr/local/bin` once the user accepts the "install command line tools"
/// prompt — so the bundle existing is a real "installed" signal that a `PATH`
/// scan alone would miss.
#[cfg(not(windows))]
const EXTRA_INSTALL_LOCATIONS: &[&str] = &[
    "/Applications/Ollama.app",
    "/usr/local/bin/ollama",
    "/opt/homebrew/bin/ollama",
];

/// What the UI tells the user when Ollama is nowhere on the machine. Written
/// for someone who will not open a terminal unless told exactly what to type.
const INSTALL_HINT: &str = "Ollama isn't installed. Download it from https://ollama.com/download \
     and open it once, or run `brew install ollama` if you use Homebrew.";

/// Everything the settings screen and first-run checklist need to know about
/// the local LLM, in one round trip. Serializes to `OllamaStatus` in
/// `src/lib/ipc.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    /// The binary (or the macOS app bundle) exists on this machine.
    pub installed: bool,
    /// The HTTP API answered. Installed-but-not-running is a normal state.
    pub running: bool,
    /// Model tags the server reports, e.g. `["qwen3:8b"]`. Empty when the
    /// server is not running — we cannot know without asking it.
    pub models: Vec<String>,
    /// Whether the model the app is configured to use is among `models`.
    pub model_ready: bool,
    /// `Some` with the next action when Ollama is not currently answering.
    pub install_hint: Option<String>,
}

/// Which downloader a [`PullProgress`] came from.
///
/// The first-run checklist tracks two separate things — "download the speech
/// models" and "install the summary AI" — that report through one progress
/// list. Without this the UI had to guess from the model's name, which breaks
/// the moment a model is renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullKind {
    /// An Ollama model, for summaries.
    Ollama,
    /// A whisper or diarization model, for transcription.
    Speech,
}

/// One progress report for a long download. Serializes to `PullProgress` in
/// `src/lib/ipc.ts`, and is the same value the UI renders for model downloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    /// Which checklist item this download belongs to.
    #[serde(default = "ollama_kind")]
    pub kind: PullKind,
    /// What is being fetched, for the label — here, the model name.
    pub name: String,
    /// 0..=100, monotonic non-decreasing across a single pull.
    pub percent: f64,
    /// Set on the terminal report when the pull failed; otherwise `None`.
    pub error: Option<String>,
    /// True on the last report, success or failure.
    pub done: bool,
}

fn ollama_kind() -> PullKind {
    PullKind::Ollama
}

/// What `/api/tags` returns.
#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagEntry>,
}

#[derive(Deserialize)]
struct TagEntry {
    name: String,
}

/// One line of the `/api/pull` NDJSON feed. Every field is optional because
/// the same stream carries manifest lines (`status` only), layer progress
/// (`total`/`completed`), the terminal `{"status":"success"}`, and failures
/// (`error` only, with no `status` at all).
#[derive(Deserialize)]
struct PullLine {
    status: Option<String>,
    error: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
}

/// Reports whether Ollama is installed, running, which models it has, and
/// whether `wanted_model` is one of them.
///
/// Never returns `Err`. A dead port, a wedged server, and something that is
/// not Ollama answering on that port all mean the same thing to the user —
/// "not running" — and the UI needs to render that, not handle it.
///
/// `installed` is probed separately from the HTTP call, because Ollama being
/// installed but not launched is the most common state on a fresh Mac and
/// needs different advice than not having it at all.
pub fn status(base_url: &str, wanted_model: &str) -> OllamaStatus {
    let tags = fetch_tags(base_url);
    let running = tags.is_some();
    let models = tags.unwrap_or_default();
    let model_ready = models.iter().any(|m| model_matches(m, wanted_model));

    // A server that answered is installed by definition, whatever the
    // filesystem probe thinks (it may be running from an unusual path).
    let installed = running || binary_present();

    OllamaStatus {
        installed,
        running,
        models,
        model_ready,
        install_hint: hint(installed, running),
    }
}

/// Downloads `model`, reporting progress as it streams.
///
/// Ollama answers `POST /api/pull` with newline-delimited JSON: a few manifest
/// lines, then many `{"status":"downloading","total":N,"completed":M}` lines,
/// then `{"status":"success"}`. Each line is parsed and reported as it
/// arrives — the response is never buffered whole, because "whole" is several
/// gigabytes and the point is a live bar.
///
/// **Malformed lines are skipped, not fatal.** A pull's authoritative outcome
/// is the terminal `success` line, so a garbled *progress* line carries no
/// information worth aborting a multi-gigabyte download over. Nothing is lost
/// by ignoring one: a stream that never reaches `success` still fails loudly
/// (see the truncated-stream path below), so skipping cannot turn a broken
/// download into a silent success.
///
/// Percent is clamped monotonic. Ollama reports `completed`/`total` per layer
/// and restarts the count at each new layer, so a raw reading would march the
/// bar backwards several times; taking the running maximum means the bar
/// holds at its high-water mark while a small layer catches up rather than
/// visibly regressing. The weights layer dominates, so in practice the bar
/// tracks it and then finishes at 100 on `success`.
pub fn pull<F: FnMut(PullProgress)>(base_url: &str, model: &str, mut on_progress: F) -> Result<()> {
    let url = format!("{base_url}/api/pull");
    let response = match ureq::post(&url)
        .config()
        .timeout_connect(Some(PULL_CONNECT_TIMEOUT))
        // Read the status ourselves so a rejected pull explains itself in
        // terms of the model name rather than as a bare HTTP code.
        .http_status_as_error(false)
        .build()
        .send_json(serde_json::json!({ "model": model, "stream": true }))
    {
        Ok(response) => response,
        Err(e) => {
            // Report before returning. Every other failure path here emits a
            // terminal progress first, and a UI driven by the callback would
            // otherwise sit at 0% forever with no reason shown — which is
            // exactly what "Ollama isn't running" looks like to a user.
            let message = format!(
                "Couldn't reach Ollama to download \"{model}\". Make sure Ollama is running, \
                 then try again."
            );
            on_progress(failed(model, &message));
            return Err(e).context(message);
        }
    };

    let http_status = response.status().as_u16();
    if !(200..300).contains(&http_status) {
        let message = format!(
            "ollama refused to download \"{model}\" (HTTP {http_status}). \
             Check the model name is spelled exactly as it appears on ollama.com."
        );
        on_progress(failed(model, &message));
        anyhow::bail!(message);
    }

    let reader = BufReader::new(response.into_body().into_reader());
    let mut highest = 0.0f64;
    let mut succeeded = false;

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                let message = format!("the download of \"{model}\" was cut off: {e}");
                on_progress(failed(model, &message));
                anyhow::bail!(message);
            }
        };

        let Ok(parsed) = serde_json::from_str::<PullLine>(line.trim()) else {
            continue; // See the doc comment: garbled progress is not fatal.
        };

        if let Some(error) = parsed.error {
            let message = format!("ollama could not download \"{model}\": {error}");
            on_progress(failed(model, &message));
            anyhow::bail!(message);
        }

        if parsed.status.as_deref() == Some("success") {
            succeeded = true;
            on_progress(PullProgress {
                kind: PullKind::Ollama,
                name: model.to_string(),
                percent: 100.0,
                error: None,
                done: true,
            });
            break;
        }

        if let Some(percent) = percent_of(parsed.completed, parsed.total) {
            if percent > highest {
                highest = percent;
            }
            on_progress(PullProgress {
                kind: PullKind::Ollama,
                name: model.to_string(),
                percent: highest,
                error: None,
                done: false,
            });
        }
    }

    if !succeeded {
        let message = format!(
            "the download of \"{model}\" ended before it finished. \
             Check your connection and try again."
        );
        on_progress(failed(model, &message));
        anyhow::bail!(message);
    }

    Ok(())
}

/// Ensures `model` is available, pulling it only if it is not already there.
///
/// Idempotent by design: safe to call on every app start and before every
/// summarization, and costs one `GET /api/tags` when the model is present.
pub fn ensure_model<F: FnMut(PullProgress)>(
    base_url: &str,
    model: &str,
    mut on_progress: F,
) -> Result<()> {
    if status(base_url, model).model_ready {
        on_progress(PullProgress {
            kind: PullKind::Ollama,
            name: model.to_string(),
            percent: 100.0,
            error: None,
            done: true,
        });
        return Ok(());
    }

    pull(base_url, model, &mut on_progress)
}

/// The terminal failure report. Always emitted before the `Err` returns, so a
/// UI driven purely by the callback shows the reason instead of a bar frozen
/// at 40%.
fn failed(model: &str, message: &str) -> PullProgress {
    PullProgress {
        kind: PullKind::Ollama,
        name: model.to_string(),
        percent: 0.0,
        error: Some(message.to_string()),
        done: true,
    }
}

/// `Some(models)` when the server answered with something we understand,
/// `None` for every kind of "not running" — connection refused, timeout, an
/// error status, or a body that is not Ollama's tag list.
fn fetch_tags(base_url: &str) -> Option<Vec<String>> {
    let url = format!("{base_url}/api/tags");
    let mut response = ureq::get(&url)
        .config()
        .timeout_global(Some(STATUS_TIMEOUT))
        .build()
        .call()
        .ok()?;

    let parsed: TagsResponse = response.body_mut().read_json().ok()?;
    Some(parsed.models.into_iter().map(|m| m.name).collect())
}

/// Whether a tag reported by the server satisfies a requested model name.
///
/// Ollama treats a bare name as `name:latest`, and `/api/tags` always reports
/// the fully qualified tag. Without this, a user who writes `qwen3` in
/// settings would be told forever that the model they just pulled is missing.
fn model_matches(present: &str, wanted: &str) -> bool {
    qualify(present) == qualify(wanted)
}

fn qualify(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

/// `completed`/`total` as a percent, or `None` for the manifest lines that
/// carry no byte counts (and for a zero total, which would divide by zero).
fn percent_of(completed: Option<u64>, total: Option<u64>) -> Option<f64> {
    let total = total?;
    let completed = completed?;
    if total == 0 {
        return None;
    }
    Some(((completed as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
}

/// Whether the Ollama binary exists anywhere we know to look.
fn binary_present() -> bool {
    if on_path(std::env::var_os("PATH").as_deref()) {
        return true;
    }
    if EXTRA_INSTALL_LOCATIONS
        .iter()
        .any(|p| Path::new(p).exists())
    {
        return true;
    }

    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Programs")
            .join("Ollama")
            .join(BINARY_NAME)
            .is_file();
    }

    false
}

fn hint(installed: bool, running: bool) -> Option<String> {
    match (installed, running) {
        (_, true) => None,
        (true, false) => Some(
            "Ollama is installed but not running. Open it and summaries will start working — nothing else needs setting up."
                .to_string(),
        ),
        (false, false) => Some(INSTALL_HINT.to_string()),
    }
}

/// Scans a `PATH`-shaped variable for an executable named `ollama`. Split out
/// from [`binary_present`] so the scan is testable without touching the
/// process environment.
fn on_path(path_var: Option<&OsStr>) -> bool {
    let Some(path_var) = path_var else {
        return false;
    };
    std::env::split_paths(path_var).any(|dir| is_executable(&dir.join(BINARY_NAME)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    /// A port nothing listens on, for the "Ollama is not running" cases.
    const DEAD_PORT: &str = "http://127.0.0.1:1";

    fn ndjson(lines: &[&str]) -> String {
        lines.join("\n")
    }

    fn tags_body(names: &[&str]) -> serde_json::Value {
        json!({ "models": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>() })
    }

    /// Collects every progress report a pull emits.
    #[allow(clippy::type_complexity)]
    fn recorder() -> (
        std::rc::Rc<std::cell::RefCell<Vec<PullProgress>>>,
        impl FnMut(PullProgress),
    ) {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        (seen, move |p: PullProgress| sink.borrow_mut().push(p))
    }

    // --- status ---------------------------------------------------------

    #[test]
    fn status_on_a_dead_port_reports_not_running_instead_of_erroring() {
        let s = status(DEAD_PORT, "qwen3:8b");

        assert!(!s.running, "nothing is listening, so it cannot be running");
        assert!(!s.model_ready);
        assert!(s.models.is_empty());
        // Whether the binary happens to exist on the test machine is not ours
        // to assert, but the hint must track it exactly.
        assert_eq!(
            s.install_hint,
            hint(s.installed, false),
            "a stopped Ollama should receive the appropriate next-step guidance"
        );
    }

    #[test]
    fn status_reports_model_ready_when_the_wanted_model_is_present() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200)
                .json_body(tags_body(&["qwen3:8b", "llama3:8b"]));
        });

        let s = status(&server.base_url(), "qwen3:8b");

        mock.assert();
        assert!(s.running);
        assert!(s.installed, "a server that answered is installed");
        assert!(s.model_ready);
        assert_eq!(s.models, vec!["qwen3:8b", "llama3:8b"]);
        assert_eq!(s.install_hint, None);
    }

    #[test]
    fn status_reports_model_not_ready_when_the_wanted_model_is_absent() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200).json_body(tags_body(&["llama3:8b"]));
        });

        let s = status(&server.base_url(), "qwen3:8b");

        assert!(s.running);
        assert!(!s.model_ready, "llama3 is not qwen3");
        assert_eq!(s.install_hint, None, "installed, just missing a model");
    }

    #[test]
    fn status_treats_a_bare_model_name_as_the_latest_tag() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200).json_body(tags_body(&["qwen3:latest"]));
        });

        assert!(
            status(&server.base_url(), "qwen3").model_ready,
            "`qwen3` in settings must match the `qwen3:latest` ollama reports"
        );
    }

    #[test]
    fn status_treats_a_non_ollama_server_as_not_running() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200)
                .body("<html>some other app owns this port</html>");
        });

        let s = status(&server.base_url(), "qwen3:8b");

        assert!(!s.running, "something answered, but it is not ollama");
        assert!(s.models.is_empty());
    }

    #[test]
    fn status_serializes_to_the_ipc_camel_case_shape() {
        let s = OllamaStatus {
            installed: true,
            running: false,
            models: vec!["qwen3:8b".into()],
            model_ready: false,
            install_hint: None,
        };

        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({
                "installed": true,
                "running": false,
                "models": ["qwen3:8b"],
                "modelReady": false,
                "installHint": null,
            })
        );
    }

    #[test]
    fn install_hint_tells_the_user_what_to_do() {
        let hint = INSTALL_HINT.to_lowercase();
        assert!(hint.contains("ollama.com/download"), "hint: {INSTALL_HINT}");
        assert!(hint.contains("brew install ollama"), "hint: {INSTALL_HINT}");
        // This sentence is rendered verbatim in the UI, and it is written
        // across a line continuation — so guard the joins, not just the words.
        assert!(!INSTALL_HINT.contains("  "), "double space: {INSTALL_HINT}");
        assert!(
            !INSTALL_HINT.contains('\n'),
            "stray newline: {INSTALL_HINT}"
        );
    }

    // --- binary detection -----------------------------------------------

    #[test]
    fn the_binary_name_carries_the_platform_extension() {
        if cfg!(windows) {
            assert_eq!(BINARY_NAME, "ollama.exe");
        } else {
            assert_eq!(BINARY_NAME, "ollama");
        }
    }

    #[test]
    fn an_installed_but_stopped_ollama_is_told_to_start_not_to_download() {
        let hint = hint(true, false).expect("a stopped Ollama needs a sentence");
        assert!(
            hint.contains("not running"),
            "must say it is not running: {hint}"
        );
        assert!(
            !hint.to_lowercase().contains("download"),
            "must not send someone who has it to download it: {hint}"
        );
    }

    #[test]
    fn a_running_ollama_needs_no_hint_at_all() {
        assert_eq!(hint(true, true), None);
    }

    #[test]
    fn an_absent_ollama_still_says_where_to_get_it() {
        let hint = hint(false, false).expect("an absent Ollama needs a sentence");
        assert!(hint.contains("ollama.com/download"));
    }

    #[test]
    fn on_path_finds_an_executable_ollama_in_a_path_directory() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join(BINARY_NAME);
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert!(on_path(Some(path_var.as_os_str())));
    }

    #[test]
    fn on_path_is_false_for_an_empty_directory_and_for_no_path_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();

        assert!(!on_path(Some(path_var.as_os_str())));
        assert!(!on_path(None));
    }

    // --- pull ------------------------------------------------------------

    #[test]
    fn pull_reports_monotonic_progress_ending_at_100_and_done() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"pulling manifest"}"#,
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":250}"#,
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":500}"#,
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":900}"#,
                r#"{"status":"verifying sha256 digest"}"#,
                r#"{"status":"success"}"#,
            ]));
        });

        let (seen, sink) = recorder();
        pull(&server.base_url(), "qwen3:8b", sink).unwrap();

        mock.assert();
        let seen = seen.borrow();
        assert_eq!(
            seen.len(),
            4,
            "one report per byte-carrying line, plus success"
        );
        assert!(
            seen.windows(2).all(|w| w[1].percent >= w[0].percent),
            "percent must never go backwards: {:?}",
            seen.iter().map(|p| p.percent).collect::<Vec<_>>()
        );
        assert_eq!(seen[0].percent, 25.0);
        assert_eq!(seen[0].name, "qwen3:8b");
        assert!(!seen[0].done);
        let last = seen.last().unwrap();
        assert_eq!(last.percent, 100.0);
        assert!(last.done);
        assert_eq!(last.error, None);
    }

    #[test]
    fn pull_never_reports_a_percent_going_backwards_across_layers() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":900}"#,
                // A second, tiny layer restarts ollama's own counter at ~0%.
                r#"{"status":"downloading","digest":"sha256:bb","total":1000,"completed":10}"#,
                r#"{"status":"success"}"#,
            ]));
        });

        let (seen, sink) = recorder();
        pull(&server.base_url(), "qwen3:8b", sink).unwrap();

        let percents: Vec<f64> = seen.borrow().iter().map(|p| p.percent).collect();
        assert_eq!(
            percents,
            vec![90.0, 90.0, 100.0],
            "the bar holds, never regresses"
        );
    }

    #[test]
    fn pull_error_line_reports_the_failure_to_the_callback_and_errors() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":400}"#,
                r#"{"error":"model not found"}"#,
            ]));
        });

        let (seen, sink) = recorder();
        let err = pull(&server.base_url(), "nope:1b", sink)
            .unwrap_err()
            .to_string();

        assert!(err.contains("nope:1b"), "error must name the model: {err}");
        assert!(
            err.contains("model not found"),
            "error must relay ollama's reason: {err}"
        );

        let seen = seen.borrow();
        let last = seen.last().unwrap();
        assert!(last.done, "the UI must not sit frozen at 40%");
        assert_eq!(last.error.as_deref(), Some(err.as_str()));
    }

    #[test]
    fn pull_skips_a_malformed_line_and_still_completes() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":300}"#,
                "}{ not json at all",
                "",
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":700}"#,
                r#"{"status":"success"}"#,
            ]));
        });

        let (seen, sink) = recorder();
        pull(&server.base_url(), "qwen3:8b", sink).unwrap();

        let percents: Vec<f64> = seen.borrow().iter().map(|p| p.percent).collect();
        assert_eq!(
            percents,
            vec![30.0, 70.0, 100.0],
            "garbage between good lines is skipped, not fatal"
        );
    }

    #[test]
    fn pull_stream_that_stops_before_success_is_an_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":400}"#,
            ]));
        });

        let (seen, sink) = recorder();
        let err = pull(&server.base_url(), "qwen3:8b", sink)
            .unwrap_err()
            .to_string();

        assert!(
            err.contains("ended before it finished"),
            "a truncated stream must not look like success: {err}"
        );
        // Also shown verbatim to the user, and also built across a line
        // continuation.
        assert!(
            err.contains("Check your connection"),
            "advice missing: {err}"
        );
        assert!(
            !err.contains("  "),
            "double space in user-facing text: {err}"
        );
        assert!(seen.borrow().last().unwrap().done);
        assert!(seen.borrow().last().unwrap().error.is_some());
    }

    #[test]
    fn pull_http_error_status_is_actionable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(404).body("model not found");
        });

        let (seen, sink) = recorder();
        let err = pull(&server.base_url(), "bogus:9b", sink)
            .unwrap_err()
            .to_string();

        assert!(err.contains("bogus:9b"), "error must name the model: {err}");
        assert!(err.contains("404"), "error should say what happened: {err}");
        assert!(seen.borrow().last().unwrap().error.is_some());
    }

    #[test]
    fn pull_against_a_dead_port_names_ollama() {
        let (seen, sink) = recorder();
        let err = pull(DEAD_PORT, "qwen3:8b", sink).unwrap_err().to_string();

        assert!(
            err.to_lowercase().contains("ollama"),
            "error must name ollama for the UI: {err}"
        );

        // And it must reach the callback, not only the return value: a UI
        // driven by progress alone would otherwise show a bar stuck at 0%
        // with nothing explaining why.
        let reports = seen.borrow();
        let last = reports
            .last()
            .expect("an unreachable server must still report a terminal failure");
        assert!(last.done, "the failure report must be terminal");
        assert!(
            last.error.as_deref().is_some_and(|e| e.contains("Ollama")),
            "the report must carry an actionable reason: {last:?}"
        );
    }

    #[test]
    fn pull_progress_serializes_to_the_ipc_camel_case_shape() {
        let p = PullProgress {
            kind: PullKind::Ollama,
            name: "qwen3:8b".into(),
            percent: 42.5,
            error: None,
            done: false,
        };

        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({
                "kind": "ollama",
                "name": "qwen3:8b",
                "percent": 42.5,
                "error": null,
                "done": false
            })
        );
    }

    // --- ensure_model ----------------------------------------------------

    #[test]
    fn ensure_model_issues_no_pull_when_the_model_is_already_present() {
        let server = MockServer::start();
        let tags = server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200).json_body(tags_body(&["qwen3:8b"]));
        });
        let pull_mock = server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(r#"{"status":"success"}"#);
        });

        let (seen, sink) = recorder();
        ensure_model(&server.base_url(), "qwen3:8b", sink).unwrap();

        tags.assert();
        assert_eq!(
            pull_mock.calls(),
            0,
            "an already-present model must not be re-pulled"
        );
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].percent, 100.0);
        assert!(seen[0].done);
        assert_eq!(seen[0].error, None);
    }

    #[test]
    fn ensure_model_pulls_when_the_model_is_missing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200).json_body(tags_body(&["llama3:8b"]));
        });
        let pull_mock = server.mock(|when, then| {
            when.method(POST).path("/api/pull");
            then.status(200).body(ndjson(&[
                r#"{"status":"downloading","digest":"sha256:aa","total":1000,"completed":500}"#,
                r#"{"status":"success"}"#,
            ]));
        });

        let (seen, sink) = recorder();
        ensure_model(&server.base_url(), "qwen3:8b", sink).unwrap();

        assert_eq!(pull_mock.calls(), 1);
        let seen = seen.borrow();
        assert_eq!(seen.last().unwrap().percent, 100.0);
        assert!(seen.last().unwrap().done);
    }

    // --- pure helpers -----------------------------------------------------

    #[test]
    fn percent_of_ignores_manifest_lines_and_zero_totals() {
        assert_eq!(percent_of(Some(1), Some(4)), Some(25.0));
        assert_eq!(percent_of(None, Some(4)), None);
        assert_eq!(percent_of(Some(1), None), None);
        assert_eq!(percent_of(Some(1), Some(0)), None, "no divide by zero");
        assert_eq!(
            percent_of(Some(9), Some(4)),
            Some(100.0),
            "clamped, never over 100"
        );
    }

    #[test]
    fn model_matches_handles_tags_in_either_direction() {
        assert!(model_matches("qwen3:latest", "qwen3"));
        assert!(model_matches("qwen3:8b", "qwen3:8b"));
        assert!(!model_matches("qwen3:8b", "qwen3"));
        assert!(!model_matches("qwen3:8b", "llama3:8b"));
    }
}
