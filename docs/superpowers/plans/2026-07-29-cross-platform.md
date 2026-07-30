# Plan C — Mac + PC + served web

**Status:** in progress, started 2026-07-29.
**Supersedes the scope of** `2026-07-27-notetaker-v2-capture.md` "Plan B2 (the Mac
day)" — B2's checklist is absorbed here rather than dropped, because the same
platform work now has to land twice (macOS and Windows) instead of once.

## What changed

Mr. Brothers asked for the app to run on **Mac, PC, and web**. Four decisions,
his, taken 2026-07-29:

| Decision | Choice |
|---|---|
| What "web" means | **The desktop app serves its own UI** over loopback/LAN. Browse the library, search, read summaries, start/stop a recording from a phone or any browser. Capture and processing stay on the machine. |
| Build order | **Windows layer + CI matrix first**, then macOS. |
| Outside code | **Lift from `fastrepl/anarlog`** (ex-Hyprnote) with attribution. |
| Windows acceleration | **CPU now, GPU later** behind a feature flag. |

### One correction on the record

The license option he was shown said "Fork Hyprnote, accept GPL-3.0". **That was
wrong** — the project renamed to `anarlog` and its `LICENSE` is plain MIT
(`Copyright (c) 2023-present Fastrepl, Inc.`). He chose it believing it cost him
the ability to sell this; it does not. It costs attribution only. Recorded here
because the decision was made on bad information that happened to point the
same way.

We are **not** adopting their 175-crate monorepo. Their capture crates sit on a
chain of internal workspace crates (`anlg-aec`, `anlg-audio-interface`,
`anlg-resampler`, …) and are async `Stream`-based, while our `AudioSource` is
deliberately synchronous and pull-based. We lift the *knowledge-dense* parts —
WASAPI device enumeration, loopback share mode, format negotiation, sample
conversion — and drain them through a ring buffer into our existing `read()`,
which is precisely what `capture/source.rs`'s own doc comment already
prescribes. The tested session loop does not change.

## Why this is cheap: the seams already exist

Plan B1 left exactly two traits with working fakes behind every platform-bound
surface, and nothing above them is platform-aware:

- `capture::source::AudioSource` — mic and system audio.
- `power::SystemProbe` — idle time and power state.

Plus three smaller platform couplings: the storage root, hardware tier
detection, and meeting-detection process names.

## The verification unlock

Nothing here can *run* macOS or Windows code, and the app crate does not even
build on Linux (`libdbus-sys` needs pkg-config we have no sudo for). Writing
two platform layers blind was the main risk in this plan. It is now much
smaller, because of one property of `cargo check`:

**`cargo check` does not link.** A crate whose dependencies are pure Rust
type-checks against a foreign target with no C toolchain and no SDK. Verified
2026-07-29 from this Linux box:

- `cargo check --target x86_64-pc-windows-msvc` — compiles real `GetLastInputInfo`
  / `GetSystemPowerStatus` calls and the `wasapi` crate. 13 s.
- `cargo check --target aarch64-apple-darwin` — compiles `objc2`,
  `objc2-screen-capture-kit` and `objc2-core-graphics`. 13 s.

**Correction, since a first pass got this wrong:** `cidre` (what anarlog uses for
Apple frameworks) does **not** cross-check. An early probe suggested it did, but
only because it was configured with no features; with `sc`/`av` enabled its build
script compiles Objective-C glue and needs a real macOS toolchain. The `objc2`
family is pure Rust and declares the runtime via `extern`, so it checks from
anywhere. That is the only reason for diverging from anarlog's binding choice,
and it is a good one — keeping the loop alive for the Mac side is worth more than
matching them.

The general rule this leaves behind: **prefer bindings with no build-script C
compilation**, and confirm every cross-check with a negative control. A clean
pass and a silently skipped target look identical.

So the platform layer goes in its own crate whose deps are pure Rust
(`wasapi`, `windows`, `objc2-*`, `cpal`, `rubato`, `ringbuf`), keeping the heavy
C++ deps (`whisper-rs`, `sherpa-rs`, bundled `rusqlite`) out of the cross-check
path. That buys a fast local loop that catches the dominant failure mode of
blind-written platform code — it doesn't compile.

**What the cross-check does not prove:** that it links, that the OS returns the
data we expect, or that any test passes. That needs real hardware, which means
CI (`windows-latest`, `macos-14`) and, for permissions and audio devices, a
human at the machine. `cargo-xwin` is installed for a fuller Windows build but
needs a `clang-cl` we have no sudo to install; not required, noted for later.

Worth knowing: **anarlog's own CI is `ubuntu-latest` only**, so the Windows code
we are adapting has never been compiled by its authors either. Our matrix is
the first thing to try it.

## Where this stands (2026-07-30, end of the second session)

**Done: 1–4, 6, 7, 8, 9, plus the scheduler wiring that was on no list at all.
Left: 5 (macOS system audio).**

Eleven commits on `claude/cross-platform-mac-pc-web-e3af6f`. Working tree clean.

**It is on GitHub now.** `George-Brothers/notetaker`, **private**, created
2026-07-30 on Mr. Brothers' word. Before that the repo had no remote at all —
the earlier note in this file blamed a broken push gate, which was wrong: the
allowlist was fine, there was simply nowhere to push. Everything is pushed;
**PR #1** carries all 81 commits since Plan A, and CI ran for the first time on
it.

Verified at the end of this session:

```
471 Rust tests (357 core, 51 platform, 47 server, 11 e2e, 5 bakeoff) + 72 frontend
   (an earlier "482" here double-counted the server's e2e binary)
clippy --all-targets -D warnings   clean
cargo fmt --check                  clean
scripts/check-platforms.sh         all three targets OK
notetaker-serve                    exercised for real over a socket, with audio
the UI                             driven in a real browser, screenshotted
```

To pick this up:

```bash
cd src-tauri
export PATH="$HOME/.cargo/bin:$PATH" LIBCLANG_PATH="$HOME/.local/lib/libclang"
cargo test -p notetaker-core -p notetaker-platform -p notetaker-server
cd .. && pnpm test && pnpm build   # pnpm build is the only typecheck
./scripts/check-platforms.sh
```

The visual loop — new this session, and the reason two layout bugs were caught —
is written up in `docs/MAP.md` under "Build environment". Short version: there
*is* a usable Chromium on this box, contrary to what every earlier note assumed.

### CI ran, and it earned its keep

See `docs/MAP.md` → "CI has run now" for the detail. Two bugs fixed (macOS
`MicSource` was `!Send`; `pnpm-workspace.yaml` had no `packages` key and broke
every job before install). One left open: **8 of 348 Windows tests**, seven in
the FLAC finalize path and one in the disk probe. Those need a Windows to
diagnose — CI is the only loop, at roughly ten minutes a cycle.

The macOS failure is worth remembering as a limit of this plan's own central
trick: **`cargo check` cross-targeting cannot catch a trait-bound error that
only appears in core**, because core is the crate that cannot be cross-checked.
The platform crate type-checked for `aarch64-apple-darwin` the whole time.

### The gap that mattered more than task 5 — closed 2026-07-30

**No production binary started the scheduler.** `Runtime::start_scheduler` was
written and tested and called by *tests only*; neither front end ever
constructed `SchedulerModels`, because nothing loaded the speech models outside
`bin/bakeoff.rs`. A recording was captured, finalized to FLAC, queued — and then
sat there forever. Every layer beneath it worked, which is exactly why it went
unnoticed for two plans: the gap was the wiring, not the engine.

It turned out to be three gaps, not one. `Runtime::start_processing` loads the
models and starts the loop; `Runtime::launch` is the single startup call both
front ends now make (`notetaker-serve` had been calling *no* startup housekeeping
at all, so its search index was whatever the previous run left behind); and
`models::ensure_segmentation_unpacked` extracts the segmentation model from the
`.tar.bz2` sherpa-onnx ships it in — nothing did, so the models could download in
full and the diarizer would still have no `.onnx` to load. Full write-up in
`docs/MAP.md` → "The scheduler, now wired".

Two things worth carrying forward:

- **Absent models are a return value, not an error.** `Processing::ModelsMissing`
  names them the way the first-run checklist does. A first launch is not a
  failure, and the app stays perfectly usable for recording and reading while
  they download.
- **A negative control caught a bad test.** The launch test passed with
  `start_up` disabled, because the SQLite index survived on disk and search
  worked without a rebuild. It now deletes the index first. Two other new tests
  were confirmed the same way and were fine — which is the point: the ratio is
  never zero.

### CI can lie, and did

A **documentation-only** commit turned all three jobs red with
`unable to find library -lonnxruntime`. `sherpa-rs`'s build script downloads its
native libraries into a per-user directory *outside* `target/`; caching `target/`
alone means cargo thinks the build script is fresh and never re-downloads, while
the linker has nothing to link against. The fix is three lines of workflow. The
habit is the lesson: **check what changed before believing a red run.**

### Task 6, as built

The shell is thirty one-line `#[tauri::command]` wrappers over
`core::dispatch` — longhand rather than macro-generated, because this crate
cannot be compiled on the dev box and boring code a human can check by eye is
the right trade when the compiler is unavailable. Three tests in **core** read
`src-tauri/src/lib.rs` as text and pin the handler list to `COMMANDS` and the
`rename_all` attributes; all three were confirmed with a negative control.
`protocol-asset` is enabled and scoped to `$HOME/Notetaker/**` so the audio
player can read a recording off disk without exposing the rest of the filesystem.

## Tasks

1. **Plan doc + MIT attribution.** ✅ This file, plus `NOTICE` naming anarlog and
   every adapted file.
2. ✅ **De-Mac-lock `notetaker-core`.** Portable default storage root (`%USERPROFILE%\Notetaker`
   on Windows, `~/Notetaker` elsewhere) — currently unwritten, `Settings::storage_root`
   defaults to `""` and the app layer never fills it. Per-OS `cpal` gating in
   `core/Cargo.toml` (macOS-only today). Add a `CpuBig` tier so a 32 GB Windows
   desktop isn't forced onto the smallest models; keep the three existing tier
   names, which are a settings-file contract. Drop the `/Users/george` hardcodes
   in `api.rs` tests.
3. ✅ **Windows `AudioSource`.** Mic via `cpal`; system audio via WASAPI loopback
   adapted from `crates/audio-actual/src/speaker/windows.rs`. Resample to
   `SAMPLE_RATE`. WASAPI loopback needs no permission prompt — Windows is the
   *easier* of the two platforms here.
4. ✅ **Windows `SystemProbe` + meeting detection.** `WinProbe` from
   `GetLastInputInfo` + `GetSystemPowerStatus` (both already compile-verified).
   Meeting detection is **ours to write** — anarlog's `detect/list/` covers
   macOS and Linux only.
5. ⬜ **macOS — PARTLY DONE.** (The only task on this list left.) The mic (`cpal`, shared with Windows) and the
   `SystemProbe` are done: `MacProbe` now takes idle time from
   `CGEventSourceSecondsSinceLastEventType` via `objc2-core-graphics`, which
   retired "verified vs assumed" risk #1 in `docs/MAP.md`. `pmset -g batt` still
   supplies AC and battery, deliberately — stable documented format, and a parse
   failure there is visible rather than silent.
   **What remains is system audio only:** ScreenCaptureKit via
   `objc2-screen-capture-kit`. Not written blind, on purpose — it needs a
   runtime-defined Objective-C delegate (`objc2::define_class!`) and the Screen
   Recording permission, and neither can be verified anywhere but on the
   hardware. The full design, and that reasoning, is in
   `platform/src/macos/speaker.rs`. Everything below it — ring buffer, downmix,
   resample — is already shared with Windows and tested.
6. ✅ **Cross-platform Tauri shell.** Thirty one-line wrappers over
   `core::dispatch`, per-OS bundling (dmg/app, msi/nsis), a real CSP, and the
   asset protocol scoped to the library folder. Pinned from Linux by three
   tests that read the shell's source — see "Task 6, as built" above.
7. ✅ **Served web UI.** HTTP server in core exposing `runtime::COMMANDS` and the
   built `dist/`, against the same contract as `src/lib/ipc.ts`. **Loopback by
   default; LAN exposure is an explicit opt-in.** The one piece fully testable
   in WSL2, so it gets real integration tests.
8. ✅ **CI matrix.** `ubuntu-latest` (core + frontend), `windows-latest`,
   `macos-14`. The only machine verification platform code will get.
9. ✅ **Update `docs/MAP.md`.** It still opens "Fully local macOS notetaker".

## Ground rules carried forward

Unchanged from Plan A/B and still binding — see `docs/MAP.md`:

- Nothing ever deletes a recording.
- The SQLite index must always be rebuildable from the files on disk.
- `~/Notetaker/Tasks/...` is a public contract. The Windows path is the same
  layout under `%USERPROFILE%`, so a folder copied between machines still opens.
- Nothing moves a live recording's folder.
- "Idle" means the recording has landed.
- Every message a user can hit is written for someone who is not an engineer.

One addition, from the served UI: **the server binds loopback unless the user
explicitly turns on LAN access.** A notetaker that quietly serves your meeting
transcripts to the coffee-shop wifi is a worse failure than any bug in it.
