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

## Where this stands (2026-07-29, end of the first session)

**Done: 1–4, 7, 8, 9. Left: 5 (macOS system audio), 6 (Tauri shell).**

Six commits on `claude/cross-platform-mac-pc-web-e3af6f`. Working tree clean,
**nothing pushed** — see the blocker below. Note the branch carries 76 commits
not on `main`, because the earlier Plan B1 work never landed there either.

Verified at the end of that session:

```
366 Rust tests (258 core, 51 platform, 52 server, 5 bakeoff) + 72 frontend
clippy --all-targets -D warnings   clean
cargo fmt --check                  clean
scripts/check-platforms.sh         all three targets OK
notetaker-serve                    exercised for real over a socket with curl
```

To pick this up:

```bash
cd src-tauri
export PATH="$HOME/.cargo/bin:$PATH" LIBCLANG_PATH="$HOME/.local/lib/libclang"
cargo test -p notetaker-core -p notetaker-platform -p notetaker-server
cd .. && pnpm test --run && pnpm build   # pnpm build is the only typecheck
./scripts/check-platforms.sh
```

**The one blocker that needs Mr. Brothers.** The CI workflow has never run, and
it is the *only* verification for `capture::platform`'s and `power::probe`'s
per-OS arms and for the Tauri app crate — none of which compile on Linux.
Running it needs a push, which fires only on his word, and the ledger separately
reports the push gate is broken for every project repo
(`gate-allowlist-missing`). Until then those files are written-and-unrun.

Recommended order when resuming: **task 6 before task 5.** The Tauri shell is
now small (`core::dispatch` already does the work, so the wrappers are thin
forwarding) and it unblocks a real app on both platforms; macOS system audio
needs the hardware regardless.

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
5. ⬜ **macOS — PARTLY DONE.** The mic (`cpal`, shared with Windows) and the
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
6. ⬜ NOT DONE — **Cross-platform Tauri shell.** The app crate is still the generated
   scaffold (a `greet` command). Needs the ~23 `#[tauri::command]` wrappers over
   `runtime::COMMANDS` — whose argument names a test already pins against
   `src/lib/ipc.ts` — plus capabilities and per-OS bundling (DMG / MSI+NSIS).
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
