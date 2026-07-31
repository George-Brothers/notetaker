# C1 — The Truth Pass: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **This run is inline.** Mr. Brothers said "no more agent use" after the
> playback run finishes. Execute these tasks in-session, one at a time, with
> the same discipline: failing test first, fix, re-run, commit.

**Goal:** The app stops being silently wrong on Windows — it writes a log, it
opens its own Settings screen, it knows the Ollama that is installed, it stops
throwing away a successful FLAC encode, and it stops promising a download it
never performs.

**Architecture:** Five bugs in dependency order. Logging is first because it is
the instrument the next two are diagnosed with — there is currently no logger
wired at all, so every `log::warn!` in the codebase is a no-op and every
failure this plan chases is invisible by construction. Two tasks (the freeze,
the download promise) deliberately specify a *method* rather than a patch: the
evidence does not exist yet, and inventing a fix without it is what produced
the work list this plan corrects.

**Tech Stack:** Rust core (`notetaker-core`), Tauri v2 app shell
(`src-tauri`), `notetaker-platform`, React/TS UI. Windows verification runs
through CI's `package-windows` artifact.

**Spec:** `docs/superpowers/specs/2026-07-30-windows-truth-design.md`

## Global Constraints

- **`pnpm build` is the only typecheck.** vitest does not typecheck. Run both.
- Rust checks run from `src-tauri/` with
  `PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang`.
  Only `cargo test -p notetaker-core` and `cargo test -p notetaker-platform`
  work here — **the `src-tauri` app crate does not build on Linux**
  (`libdbus-sys` needs pkg-config we have no sudo for). Changes to
  `src-tauri/src/lib.rs` are verified by `scripts/check-platforms.sh` and by
  CI, never by a local build.
- Clippy with `--all-targets`, or test code is never linted.
- `scripts/check-platforms.sh` before every push.
- `notetaker-platform` depends on no other notetaker crate. That property is
  what lets Linux cross-check the Windows and macOS code at all.
- **No modals, no blocking, no disabled buttons.** Say the true thing once and
  get out of the way. Every user-visible sentence is written for someone who
  is not an engineer.
- **Nothing deletes or rewrites audio.** Task 3 makes deletion *less* eager,
  never more.
- A Windows-only path that cannot be unit-tested on Linux is **marked
  unverified** until it has been run on the real machine. Never described as
  working before then.
- **Getting a build onto the PC requires a push**, because the installer comes
  from CI's `package-windows` job (`docs/MAP.md` has the download command).
  Push only on Mr. Brothers' explicit word.

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/core/src/logging.rs` (create) | Where the log file goes, and how it is set up. Portable, testable. |
| `src-tauri/Cargo.toml` (modify) | The logger implementation dependency. |
| `src-tauri/src/lib.rs` (modify) | Calls the logger setup before anything else; exposes `open_log_folder`. |
| `src-tauri/core/src/runtime.rs` (modify) | `log_path` command; capture-note on a failed WAV cleanup. |
| `src-tauri/core/src/ollama/mod.rs` (modify) | Windows binary name, Windows install locations, the stopped-not-missing sentence. |
| `src-tauri/core/src/capture/flac.rs` (modify) | A verified FLAC is a success even when the WAV will not delete. |
| `src/components/FirstRun.tsx` (modify) | Stop promising a download that does not happen. |
| `src/lib/ipc.ts` (modify) | The two new commands. |
| `docs/MAP.md`, `docs/HANDOVER.md` (modify, **last**) | Both are also touched by the playback branch — do these after it lands. |

---

### Task 1: A log file, because nothing else can be diagnosed without one

`log = "0.4"` is a dependency of the app crate and **no logger is ever
initialized** — there is no `env_logger`, no `tauri-plugin-log`, no
`set_logger` call anywhere. Every `log::info!`, `log::warn!` and `log::error!`
in this codebase is discarded at runtime. `%LOCALAPPDATA%\Notetaker` contains
binaries and nothing else, which is the same fact seen from the other side.

Two of the bugs below are diagnosed by reading this log. It goes first.

**Files:**
- Create: `src-tauri/core/src/logging.rs`
- Modify: `src-tauri/core/src/lib.rs` (add `pub mod logging;`)
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/core/src/runtime.rs` (the `COMMANDS` list and a `log_path` method)
- Modify: `src/lib/ipc.ts`

**Interfaces:**
- Produces:
  ```rust
  /// The directory logs are written to, given the app data dir.
  pub fn log_dir(app_data: &Path) -> PathBuf;
  /// The current log file inside it.
  pub fn log_file(app_data: &Path) -> PathBuf;
  /// Wires the global logger. Idempotent; never fails the app.
  pub fn install(app_data: &Path) -> PathBuf;
  ```
  and on `Runtime`: `pub fn log_path(&self) -> String`.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/core/src/logging.rs` with only its tests, then watch them
fail to compile:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_land_in_a_logs_directory_under_the_app_data_dir() {
        let base = Path::new("/tmp/appdata");
        assert_eq!(log_dir(base), base.join("logs"));
        assert_eq!(log_file(base), base.join("logs").join("notetaker.log"));
    }

    #[test]
    fn installing_twice_is_not_an_error() {
        // start_up and the serve binary both call this; a second call must
        // be a no-op rather than a panic that takes the app down.
        let dir = tempfile::tempdir().unwrap();
        let first = install(dir.path());
        let second = install(dir.path());
        assert_eq!(first, second);
    }

    #[test]
    fn a_log_directory_that_cannot_be_created_does_not_panic() {
        // Logging is diagnostic scaffolding. It must never be the reason
        // the app fails to open.
        let file = tempfile::NamedTempFile::new().unwrap();
        // A *file* where the app data dir should be: creating `logs/`
        // beneath it is impossible.
        let _ = install(file.path());
    }

    #[test]
    fn something_logged_after_install_reaches_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = install(dir.path());
        log::warn!("canary");
        flush();
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            body.contains("canary"),
            "a warning logged after install must be in {}, got: {body}",
            path.display()
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core logging
```

Expected: FAIL to compile — `log_dir`, `log_file`, `install`, `flush` do not
exist.

- [ ] **Step 3: Add the dependency**

In `src-tauri/core/Cargo.toml`, add a logger that writes to a file and rolls.
Prefer one already in the lock file if there is one; otherwise:

```toml
fern = "0.7"
```

`fern` is chosen over `tauri-plugin-log` deliberately: the log setup must live
in `notetaker-core`, which both the Tauri shell **and** `notetaker-serve` link,
so a Tauri-only plugin would leave the served build silent. Check
`Cargo.lock` first — if `env_logger` or `flexi_logger` is already present
transitively, use that instead of adding a third logging crate.

- [ ] **Step 4: Write the implementation**

```rust
//! Where this app's log goes, and how it gets there.
//!
//! Until 2026-07-30 there was no logger at all: `log = "0.4"` was a
//! dependency, nothing ever called `set_logger`, and so every `log::warn!` in
//! the codebase — including the one that records a FLAC finalize giving up —
//! was discarded. A week of "the app is silently wrong on Windows" was
//! invisible for exactly that reason.
//!
//! Lives in `notetaker-core` rather than the Tauri shell so that
//! `notetaker-serve` gets the same log from the same code. A Tauri-only
//! plugin would have left the served build silent.

use std::path::{Path, PathBuf};

/// The directory logs are written to.
pub fn log_dir(app_data: &Path) -> PathBuf {
    app_data.join("logs")
}

/// The current log file.
pub fn log_file(app_data: &Path) -> PathBuf {
    log_dir(app_data).join("notetaker.log")
}

/// Wires the global logger and returns the file it writes to.
///
/// **Never fails.** Logging is diagnostic scaffolding; an app that refuses to
/// open because it could not create a log directory has turned a convenience
/// into an outage. Every failure path here degrades to "no file logging" and
/// returns the path it would have used.
pub fn install(app_data: &Path) -> PathBuf {
    let path = log_file(app_data);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return path;
        }
    }
    // ... fern dispatch: level Info, timestamp + level + target + message,
    // chain to the file, `.apply()` ignored on Err (already installed).
    path
}

/// Flushes buffered records. Called before the app exits and by tests.
pub fn flush() { /* ... */ }
```

Write the real `fern` body; the comment sketch above is the shape, not a
placeholder to leave in.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core logging
```

Expected: PASS, 4 tests.

- [ ] **Step 6: Wire it into both binaries**

In `src-tauri/src/lib.rs`, inside the Tauri setup and **before** the runtime is
built, resolve the app data dir (`app.path().app_data_dir()`) and call
`notetaker_core::logging::install(&dir)`. Do the same in `notetaker-serve`'s
entry point. Then add the command so the UI can point at it:

- `Runtime::log_path(&self) -> String` returning `logging::log_file(...)` as a
  string;
- `"log_path"` in `runtime.rs`'s `COMMANDS`;
- a match arm and the test list entry in `dispatch.rs`;
- a `#[tauri::command]` wrapper and `generate_handler!` entry in
  `src-tauri/src/lib.rs`;
- `logPath: () => invoke<string>("log_path")` in `src/lib/ipc.ts`.

Miss one and a named drift test tells you which.

- [ ] **Step 7: Add "Open the log folder" to Settings**

`tauri-plugin-opener` is already a dependency. In `src/components/Settings.tsx`,
under the existing Storage section, add a button that calls `logPath()` and
opens its parent. Copy, verbatim:

> Open the log folder — if something goes wrong, this is what to send.

- [ ] **Step 8: Full check**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo clippy -p notetaker-core --all-targets -- -D warnings
```

then

```bash
pnpm test --run && pnpm build && pnpm lint && bash scripts/check-platforms.sh
```

Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "feat: a log file — the instrument every other fix on this list needs"
```

---

### Task 2: Ollama that is installed is not "not installed"

His machine has `C:\Users\georg\AppData\Local\Programs\Ollama\ollama.exe`, and
it is on `PATH`. The app tells him to download Ollama. His words: *"i DO have
ollama its just not running so idk why i would download it again."*

`BINARY_NAME` is `"ollama"`, so on Windows `dir.join(BINARY_NAME)` is
`...\Ollama\ollama` — a file that does not exist. `EXTRA_INSTALL_LOCATIONS`
holds three macOS paths. `binary_present()` cannot return true on Windows.

`OllamaStatus` already carries `installed` and `running` separately and already
returns the model list. The state was available; the app got one boolean wrong
and then said the wrong sentence.

**Files:**
- Modify: `src-tauri/core/src/ollama/mod.rs` (:37-55, `binary_present`, `status`)
- Modify: `src/components/Settings.tsx` and `src/components/FirstRun.tsx` (the hint text)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `OllamaStatus.install_hint` now means "what to do about Ollama",
  set for both the missing case and the installed-but-stopped case; `None`
  only when Ollama is actually answering.

- [ ] **Step 1: Write the failing tests**

In `ollama/mod.rs`'s `mod tests`:

```rust
#[test]
fn on_path_finds_ollama_exe_on_windows() {
    // The exact shape of Mr. Brothers' machine: Ollama installed under
    // AppData\Local\Programs and on PATH, as `ollama.exe`. The old
    // `dir.join("ollama")` could never match it, so the app told him to
    // download software he already had.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(BINARY_NAME), b"binary").unwrap();
    let path_var = std::ffi::OsString::from(dir.path());
    assert!(on_path(Some(path_var.as_os_str())));
}

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
    let status = OllamaStatus {
        installed: true,
        running: false,
        models: vec![],
        model_ready: false,
        install_hint: hint(true, false),
    };
    let hint = status.install_hint.expect("a stopped Ollama needs a sentence");
    assert!(
        hint.contains("not running"),
        "must say it is not running, got: {hint}"
    );
    assert!(
        !hint.to_lowercase().contains("download"),
        "must not tell someone who has it to download it, got: {hint}"
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
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core ollama
```

Expected: FAIL — `hint` does not exist, and
`the_binary_name_carries_the_platform_extension` fails on the `windows` arm
under `check-platforms.sh`.

- [ ] **Step 3: Write the fix**

```rust
/// The executable's name. Windows needs the extension — without it the
/// `PATH` scan below cannot match a real install, which is exactly how a
/// machine with Ollama on `PATH` was told to go and download Ollama.
#[cfg(windows)]
const BINARY_NAME: &str = "ollama.exe";
#[cfg(not(windows))]
const BINARY_NAME: &str = "ollama";

/// Places Ollama lands that are not necessarily on `PATH`.
///
/// The macOS app bundle keeps its CLI inside itself and only symlinks it into
/// `/usr/local/bin` once the user accepts the "install command line tools"
/// prompt — so the bundle existing is a real "installed" signal a `PATH` scan
/// alone would miss. The Windows installer writes under the user's
/// `AppData\Local\Programs`, which is on `PATH` in the installing shell but
/// not always in a process launched from elsewhere.
#[cfg(windows)]
const EXTRA_INSTALL_LOCATIONS: &[&str] = &[
    r"C:\Program Files\Ollama\ollama.exe",
];
#[cfg(not(windows))]
const EXTRA_INSTALL_LOCATIONS: &[&str] = &[
    "/Applications/Ollama.app",
    "/usr/local/bin/ollama",
    "/opt/homebrew/bin/ollama",
];
```

On Windows also probe `%LOCALAPPDATA%\Programs\Ollama\ollama.exe` by reading
`LOCALAPPDATA` at runtime, since it is per-user and cannot be a `const`.

Then split the sentence out of `status()`:

```rust
/// What to tell the user about Ollama, given what we found.
///
/// Three states, three sentences. The middle one is the one that was missing:
/// someone whose Ollama is installed but stopped needs "open it", and telling
/// them to download it again reads as the app not knowing what is on their
/// own machine.
fn hint(installed: bool, running: bool) -> Option<String> {
    match (installed, running) {
        (_, true) => None,
        (true, false) => Some(
            "Ollama is installed but not running. Open it and summaries will \
             start working — nothing else needs setting up."
                .to_string(),
        ),
        (false, false) => Some(INSTALL_HINT.to_string()),
    }
}
```

and use it in `status()`.

- [ ] **Step 4: Run the tests**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core && bash ../scripts/check-platforms.sh
```

Expected: PASS, including the Windows arm under `check-platforms.sh`.

- [ ] **Step 5: Make the UI say it**

`FirstRun.tsx` item 4 and `Settings.tsx`'s Summary AI section both render the
hint. Confirm both show `installHint` when present rather than hardcoding
"isn't installed", and that a stopped Ollama does not offer a Download button.
Add a frontend test in the style of `SetupNotice.test.ts` asserting the
stopped-state wording, since here the wording is the feature.

- [ ] **Step 6: Full check and commit**

```bash
pnpm test --run && pnpm build && pnpm lint
git add -A && git commit -m "fix: an Ollama that is installed but stopped is told to start, not to download"
```

---

### Task 3: A verified FLAC is a success, even when the WAV will not delete

Two of his three recordings kept both files. `finalize_to_flac` encodes,
verifies, and only then removes the WAV — so both surviving means the encode
**and** the verify succeeded and `std::fs::remove_file` failed. The capture
thread's handle was still open, which Linux permits and Windows refuses.

The function then returns `Err` on a success, and `compress_tracks` swallows it
into a `log::warn!` that (before Task 1) went nowhere.

Three wrongs: a delete that fails, a success reported as a failure, and a
failure reported to no one. This task fixes all three without ever making
deletion more eager.

**Files:**
- Modify: `src-tauri/core/src/capture/flac.rs:47-80`
- Modify: `src-tauri/core/src/runtime.rs` (`compress_tracks`)

**Interfaces:**
- Produces:
  ```rust
  /// What a finalize actually did.
  pub struct Finalized {
      pub flac: PathBuf,
      /// The WAV that could not be removed, if one survives.
      pub wav_kept: Option<PathBuf>,
  }
  pub fn finalize_to_flac(wav_path: &Path, keep_wav: bool) -> Result<Finalized>;
  ```
  `Err` now means only what it always claimed to mean: the audio did not make
  it into a verified FLAC.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_wav_that_cannot_be_deleted_is_still_a_successful_encode() {
    // Windows refuses to unlink a file whose handle is still open; Linux
    // allows it, which is why this shipped. A read-only parent directory
    // reproduces the same refusal here.
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("audio-mic.wav");
    write_wav(&wav, &tone(0.5, 0.3));

    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(dir.path(), perms).unwrap();

    let result = finalize_to_flac(&wav, false);

    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(dir.path(), perms).unwrap();

    let done = result.expect("a verified FLAC is a success whatever the delete did");
    assert!(done.flac.exists(), "the FLAC must survive");
    assert_eq!(
        done.wav_kept.as_deref(),
        Some(wav.as_path()),
        "the surviving WAV must be reported, not silently left"
    );
}
```

If a read-only directory does not block the unlink in this environment (some
filesystems and a root-ish test runner will not), skip that mechanism and
inject the failure instead: give `finalize_to_flac` a private inner function
taking a `remove: impl Fn(&Path) -> io::Result<()>` and test the inner one with
a closure that returns `ErrorKind::PermissionDenied`. Do **not** delete the
test — pick whichever mechanism actually reproduces the refusal here.

- [ ] **Step 2: Run it to verify it fails**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core flac
```

Expected: FAIL — the current code returns `Err` from the `remove_file`
`?`, and `Finalized` does not exist.

- [ ] **Step 3: Write the fix**

Replace the tail of `finalize_to_flac`:

```rust
    // The encode is verified: the audio is safe in the FLAC. Whether the WAV
    // goes away is a disk-space question, and a disk-space question must
    // never be reported as a lost recording.
    //
    // Windows refuses to unlink a file whose handle is still open. One retry
    // after a short pause covers the common case — the capture thread closing
    // moments later — and anything still holding it after that is reported to
    // the user rather than argued with.
    let mut wav_kept = None;
    if !keep_wav {
        if let Err(e) = remove_with_one_retry(wav_path) {
            log::warn!("keeping {} as wav: {e:#}", wav_path.display());
            wav_kept = Some(wav_path.to_path_buf());
        }
    }
    Ok(Finalized { flac: flac_path, wav_kept })
```

with

```rust
/// Removes a file, retrying once after a short pause.
///
/// Purely for Windows: a handle the capture thread has not closed yet makes
/// the first attempt fail and the second succeed. On Unix the first attempt
/// succeeds and the retry never runs.
fn remove_with_one_retry(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(250));
            std::fs::remove_file(path)
        }
    }
}
```

Update the three existing call sites — `recover.rs:282`, `runtime.rs:1544`, and
any test — for the new return type.

- [ ] **Step 4: Tell the user, in `compress_tracks`**

```rust
    fn compress_tracks(&self, rec: &RecordingRef) {
        // ...
            match finalize_to_flac(&wav, keep_wav) {
                Ok(done) => {
                    if let Some(kept) = done.wav_kept {
                        // `capture_note` and not `error`: this describes the
                        // *audio on disk*, it outlives every processing
                        // attempt, and nothing about it stops a transcript.
                        self.add_capture_note(
                            rec,
                            "This recording is saved twice — a compressed copy and \
                             the original. Both play fine; the original just takes \
                             more space. You can delete the .wav file if you want \
                             the room back.",
                        );
                        log::warn!("could not remove {}", kept.display());
                    }
                }
                Err(e) => log::warn!("keeping {} as wav: {e:#}", wav.display()),
            }
    }
```

Use whatever the existing helper for appending a `capture_note` is — `session.rs`
already joins notes on stop; follow that pattern rather than inventing a second
one. Add a `runtime.rs` test proving the note reaches `get_recording`'s
`captureNote`.

- [ ] **Step 5: Run everything and commit**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo clippy -p notetaker-core --all-targets -- -D warnings
git add -A && git commit -m "fix: a verified FLAC is a success even when the WAV will not delete"
```

---

### Task 4: The Settings freeze — diagnose, then fix

**This task has no patch, on purpose.** The evidence does not exist yet.
Inventing one is precisely what produced the work list this plan corrects.

What is known: the app is responsive before the settings gear is clicked and
`Not Responding` about thirteen seconds after, reproducibly, on a fresh launch.
Settings mounts exactly three commands — `getSettings`, `detectedTier`,
`ollamaStatus` — and nothing else. `detectedTier` is a `sysinfo` memory read
plus a core count. `getSettings` reads one small JSON file. `ollamaStatus`
makes an HTTP call with a three-second `timeout_global`.

`ollamaStatus` is the suspect. **Three seconds is not thirteen**, so it is not
yet the answer.

- [ ] **Step 1: Instrument the three commands**

With Task 1's logger in place, add an `log::info!` on entry and on exit, with
elapsed millis, to `get_settings`, `detected_tier` and `ollama_status` in
`runtime.rs`. One line each. This is the measurement, and it stays in
afterwards — a command that can hang the UI is worth timing forever.

- [ ] **Step 2: Get a build onto the machine**

Push (**on Mr. Brothers' word only**), wait for CI's `package-windows` job,
then:

```bash
gh run download <run> --name notetaker-windows-installer --dir /mnt/c/Users/georg/Downloads/notetaker
```

Install and launch it from WSL via `powershell.exe`.

- [ ] **Step 3: Reproduce and read the log**

Click the settings gear, wait twenty seconds, confirm `Responding` is false,
then read `%LOCALAPPDATA%\Notetaker\logs\notetaker.log`. The command with an
entry line and no exit line is the one that hangs.

If all three log an exit, the hang is in the webview or the IPC layer, not in
a command — say so plainly and re-scope rather than forcing the hypothesis.

- [ ] **Step 4: Write a failing test for the actual cause, then fix it**

Whatever the log names, the fix gets a test that fails without it. If the
cause is `ollamaStatus` blocking the main thread, the test is that the command
is declared `async` and does not run on the UI thread; if it is a `ureq` call
whose `timeout_global` does not cover connect on Windows, the test is against
a dead port with a wall-clock assertion. **Write the test the evidence calls
for, not the one this paragraph guesses at.**

- [ ] **Step 5: Verify on the real machine, then commit**

Re-build, re-install, re-run, click Settings, confirm the screen loads and the
app stays responsive. Only then is this task done — it cannot be closed by a
green Linux suite.

- [ ] **Step 6: Report what Settings actually looks like**

Screenshot the working Settings screen and compare it against Mr. Brothers'
four complaints. This is the input C2 is waiting on: three of his four items
may already be built and were simply never visible.

---

### Task 5: Stop promising a download that never happens

The first-run panel says *"Happens automatically in the background — or
download now."* It does not happen automatically. `start_processing` returns
`Processing::ModelsMissing`, logs "not transcribing yet", and stops. The only
thing that ever downloads a speech model is the user pressing the button.

His three recordings sat at `Queued` for a week because of this sentence.

**This needs one decision from Mr. Brothers** before the code is written, and
the plan does not get to make it:

- **(a) Make the sentence true** — start the download automatically on first
  launch. The app then does its job without being asked. Costs ~1.9 GB of
  someone's bandwidth unannounced, which is the thing his "don't force it" rule
  exists to prevent.
- **(b) Make the sentence honest** — *"Nothing is transcribed until these are
  downloaded. It takes about 1.9 GB."* Plus: a recording stuck at `Queued`
  should say **why**, right there in the library row, rather than sitting there
  looking like it is working.

**(b) is the recommendation**, because it is the one that matches his stated
rule, and because the deeper bug is not the download — it is a row that says
`Queued` forever and never explains itself.

- [ ] **Step 1: Get the decision.** Do not proceed without it.
- [ ] **Step 2: Write the failing test** for whichever was chosen. For (b):
      a recording with status `recorded` and missing models renders the reason
      in its library row and in the note, in the `SetupNotice.test.ts` style —
      wording asserted, because the wording is the feature.
- [ ] **Step 3: Implement, run `pnpm test --run && pnpm build && pnpm lint`, commit.**

---

### Task 6: Prove it on the real machine, and write it down

- [ ] **Step 1: Full local suite**

```bash
cd src-tauri && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo test -p notetaker-core && PATH=$HOME/.cargo/bin:$PATH LIBCLANG_PATH=$HOME/.local/lib/libclang cargo clippy -p notetaker-core --all-targets -- -D warnings
pnpm test --run && pnpm build && pnpm lint && bash scripts/check-platforms.sh
```

- [ ] **Step 2: Build, install, and walk the five findings on his PC**

One pass, each item confirmed or not:

1. `%LOCALAPPDATA%\Notetaker\logs\notetaker.log` exists and has content.
2. Settings opens, loads, and the app stays responsive.
3. The Ollama section says *installed but not running* — not "download it".
4. A fresh recording leaves one file per track, or a `capture_note` saying why
   it did not.
5. A queued recording says why it is queued.

- [ ] **Step 3: Update the docs — after the playback branch has landed**

`docs/MAP.md` and `docs/HANDOVER.md` are both touched by the playback work.
Do this last to avoid a conflict.

In `MAP.md`: add logging to the layout, and move the Ollama line out of
"assumed" into "verified" — it will have been checked against a real install
for the first time.

In `HANDOVER.md`: replace the §5 "two capture bugs" section with what was
actually found, and strike the four wrong claims listed in the spec. A
handover that is wrong in four places is worse than none.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "docs: the truth pass, verified on the real machine"
```

---

## What this plan does not do

- **Signing the installer.** Costs money; his call.
- **C2 — the settings features.** Mic picker, model status, Ollama model
  dropdown, open-folder button. Planned separately once Task 4 Step 6 shows
  what the Settings screen actually contains.
- **The library showing a live capture as `Recorded`.** Cosmetic; already
  judged not worth an on-disk contract change.
- **Root-causing the 44-byte system WAV.** WASAPI loopback with nothing
  playing produces no frames, which is correct behaviour. The playback branch
  already stops it being offered as a track.
