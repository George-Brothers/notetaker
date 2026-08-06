# MAP — Notetaker (personal)

A fully local notetaker for Mr. Brothers, on **Mac and PC**, with the same UI
also **served to a browser** so the library can be read from a phone. High-quality
dual-track recording now, idle-time local transcription (EN/ZH, Speaker 1/2/3)
and summarization later, organized by tasks. No cloud, ever.

**In flight? Read `docs/HANDOVER.md` first** — branches, what is done, what
is next, and what is waiting on Mr. Brothers.

## State
- **UI overhaul "lit from within" (2026-08-04): COMPLETE on branch
  `claude/app-ui-ux-overhaul-96e4c6`.** Aurora token system (dark "luminous
  glass" / light "porcelain"), Echo icon + tray with recording state,
  close-to-tray, global hotkeys (`CommandOrControl+Alt+N` /
  `CommandOrControl+Alt+Space` — Ctrl on Windows, Cmd on macOS — rebindable
  in Settings → Hotkeys), six-section Settings with mic picker and folder
  picker, library sort/filter, find-and-jump palette, custom titlebar
  (frameless, 500px minimum width). Spec:
  `docs/superpowers/specs/2026-08-04-ui-overhaul-design.md`. Before/after
  renders in `docs/superpowers/specs/assets/2026-08-04-pitch/`. Still owed
  from real hardware: the Windows interactive pass below.
- **Plan A (portable core): COMPLETE** (2026-07-23) — storage, index, queue,
  pipeline, models, UI library view.
- **Plan B1 (everything not platform-locked): COMPLETE** (2026-07-27). Capture
  engine, crash recovery + FLAC, meeting watcher, idle/power gating, Ollama
  manager, the `Runtime` facade, and the remaining UI. Its independent review
  and what remains open: `docs/superpowers/specs/2026-07-27-planb1-review.md`.
- **Plan C (Mac + PC + served web): IN PROGRESS** on branch
  `claude/cross-platform-mac-pc-web-e3af6f`.
  Done: portable paths, the `CpuBig` tier, the `notetaker-platform` crate,
  the **complete Windows layer** (WASAPI loopback, cpal mic, `WinProbe`,
  meeting detection), the core adapter, the **served web UI**, CI, the
  **Granola-shaped frontend** with the live notepad, the **Tauri shell**, and
  (2026-07-30) the **scheduler wiring** — recordings are now actually
  transcribed instead of queueing forever — and the **Windows installer**, built
  by CI on every push.
  **Plan C is COMPLETE as of 2026-08-05**: macOS system audio (ScreenCaptureKit)
  is written and **verified capturing real audio on a real Mac**, along with
  Metal, the `.app` bundle, and the microphone permission string. See
  "The Mac day" below.
  Plan and decisions: `docs/superpowers/plans/2026-07-29-cross-platform.md`.

## Where this lives
- **GitHub: `George-Brothers/notetaker`, private.** Created 2026-07-30 on Mr.
  Brothers' word; before that the repo had no remote at all and nothing had ever
  been pushed.
- Branches: `main` (Plan A spec only, one commit), the working branch above,
  plus `plan-a-core` and `plan-b-capture` kept for the record.
- **PR #1** carries everything since Plan A. Opened for the record and to give
  CI something to run on, not for line-by-line review of 81 commits.
- **Spec:** `docs/superpowers/specs/2026-07-23-notetaker-design.md`.
- **Bake-off:** `docs/superpowers/specs/bakeoff-result.md` — SenseVoice beats
  Whisper-tiny on Chinese. Re-run against `large-v3-turbo` on the Mac.

## Layout (as built)
Four Rust crates under `src-tauri/`, and the dependency direction between the
first two is load-bearing — see "How this is verified".

- **`core/`** — `notetaker-core`: all portable logic. Owns the traits.
  - `capture/` — `session`, `track`, `flac`, `recover`, `source` (the
    `AudioSource` seam + fakes), `platform` (the adapter onto the real devices).
  - `paths` (OS-correct directories), `dispatch` (one JSON entry point into
    `Runtime`, shared by every transport), `storage`, `index` (SQLite FTS5,
    CJK-segmented), `queue`, `pipeline/`, `models`, `watch/`, `power/`,
    `ollama/`, `scheduler`, `api`, `runtime`.
  - The notepad layer: `notes` (`notes.md`, the user's own words),
    `templates` (the shape a summary is written to), `actions` (the checklist,
    parsed out of `summary.md`), `transcript` (the `[HH:MM:SS] **Name:**` lines
    parsed back into timed segments), `pipeline/ask` (one question about one
    recording).
- **`platform/`** — `notetaker-platform`: the per-OS devices. **Depends on no
  other notetaker crate**, and only on pure-Rust libraries.
  - `convert`, `resample`, `ring` — pure, platform-independent, fully tested.
  - `mic` (cpal, both platforms), `windows/` (WASAPI loopback + Win32 power),
    `macos/` (CoreGraphics idle; ScreenCaptureKit system audio, **written and
    run** 2026-08-05).
  - `examples/` — `system-audio` and `microphone`. Not demos: they are the only
    way to answer "does a real device produce a sample", which no unit test and
    no CI runner can. Run them on hardware after touching capture.
- **`server/`** — `notetaker-server`: serves the UI over HTTP.
  `notetaker-serve` is a working Notetaker on a PC with no Tauri at all.
- **`.` (app crate)** — the Tauri shell: thirty one-line `#[tauri::command]`
  wrappers over `core::dispatch`, so the desktop app and the served UI run the
  same code. Does not build on Linux (`libdbus-sys` needs pkg-config we have no
  sudo for), so a **test in core reads `src-tauri/src/lib.rs`** and fails if the
  handler list drifts from `COMMANDS` or a camelCase argument loses its
  `rename_all`. That is the only check of this crate that runs here.
- `src/` — React/TS UI, in Granola's shape: a left rail that is both the
  navigation and the library, and a note that is your own typing at full
  contrast above the AI's expansion of it in grey.
  - `lib/ipc.ts` is the contract; `lib/transport.ts` switches between Tauri IPC
    and HTTP, and owns `audioSrc` — the one place the two transports genuinely
    differ rather than carrying the same JSON.
  - `NoteView` owns the one audio player for a recording; `TranscriptPanel`
    consumes it for timestamp seeking and active-line highlighting.
  - Tailwind v4 + Radix + Lucide + cmdk + a self-hosted Inter. Every colour is
    a token in `styles/theme.css`; **no component may hardcode one**, or the
    dark theme silently loses that element. `styles/panels.css` styles Settings
    and the first-run checklist, which keep their tested markup.
  - `components/ui.tsx` holds the shared primitives.
- `fixtures/` — `bilingual.wav`, `diarization-check.wav`, reference transcript.

## How this is verified — read this before trusting anything
**Read this section knowing which machine you are on.** It was written from a
Linux box that could not run macOS or Windows code at all, and every technique
below exists to work around that. Since 2026-08-05 there is **a Mac**, where
none of the workarounds are needed: it builds all four crates natively in about
30 seconds, runs the full suite in two, and can open a real microphone. On a
Mac, prefer running the thing to reasoning about it.

The cross-check still matters for Windows, and from Linux. The property that
makes writing foreign code defensible is: **`cargo check` does not link**, so a
crate with pure-Rust dependencies type-checks against a foreign target with no
cross-compiler and no SDK.

That is why `notetaker-platform` depends on no other notetaker crate. Core pulls
in bundled SQLite, whisper.cpp and sherpa-onnx; **core cannot be cross-checked at
all** — verified, it fails looking for MSVC's `lib.exe`.

```bash
scripts/check-platforms.sh      # all three targets, ~30s
```

| Layer | How far it is verified |
|---|---|
| `convert`, `resample`, `ring` | **Fully tested on Linux.** Deliberate — this is where capture bugs live, and they all produce plausible *audio* rather than an error |
| `macos/speaker` (system audio), `mic` on a Mac | **Run on real hardware** (2026-08-05) — real samples, non-zero peak, nothing dropped. See "The Mac day" |
| `windows/` | **Compile-verified** for the real target. Mic confirmed by a real recording; loopback wrote 0 bytes and is unexplained |
| `capture::platform`, `power::probe` per-OS arms | **CI only.** Never compiled on Linux |
| The Tauri app crate | **CI only.** Never compiles on Linux |
| `notetaker-server` + `dispatch` | **Fully verified here**, including a real binary over a real socket |
| The whole UI, visually | **Seen and screenshotted** (2026-07-30) — real binary, real files, real audio, in Chrome. See below |
| The scheduler wiring | **Decisions tested; the happy path is not.** A real model load is not a unit test — see "The scheduler, now wired" |
| Speech routing | **Measured on real bilingual meetings** (2026-07-30), both models loaded, against a Whisper-only baseline. See `specs/bakeoff-result.md` |
| The Windows installer | **Installed and launched on a real PC** (2026-07-30). The app starts and renders — see "The installer" |

The cross-check was itself confirmed with a negative control: a deliberate type
error in `windows/power.rs` *is* caught.

## Build environment (macOS)
Set up from nothing on 2026-08-05 — the machine had no Rust at all. Homebrew's
`rustup` keeps its shims in a keg, so the PATH entry is not `~/.cargo/bin`:

```bash
brew install rustup cmake && rustup default stable
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # not ~/.cargo/bin
npm install -g pnpm
```

- **Xcode is not required.** Command Line Tools carry the ScreenCaptureKit SDK,
  and whisper.cpp embeds its Metal shaders as source, so the missing
  `xcrun metal` never matters. See "The Mac day" for the one thing it may cost.
- `cargo test -p notetaker-core -p notetaker-platform -p notetaker-server` from
  `src-tauri/` — no `LIBCLANG_PATH`, no `LD_LIBRARY_PATH`, nothing. ~2 s warm.
- `pnpm tauri build` produces `.app` and `.dmg` under
  `src-tauri/target/release/bundle/`. It then **fails** on updater signing
  unless `TAURI_SIGNING_PRIVATE_KEY` is set; both bundles are already written by
  that point.
- After changing anything in capture, run the two examples on hardware —
  `cargo run -p notetaker-platform --example system-audio` and
  `--example microphone`. Nothing else can catch a stream that starts and
  records silence.

## Build environment (WSL2, hard-won)
- `cargo test -p notetaker-core -p notetaker-platform -p notetaker-server` from
  `src-tauri/`. Needs `PATH=$HOME/.cargo/bin` and
  `LIBCLANG_PATH=$HOME/.local/lib/libclang`.
- **`pnpm build` is the only typecheck** — vitest does not typecheck, so a
  contract change can pass every test and still be broken. Run both.
- Clippy with `--all-targets`; without it, test code is never linted.
- Running `notetaker-serve` by hand needs
  `LD_LIBRARY_PATH=src-tauri/target/debug` for sherpa's shared library.
- `models/` is gitignored; fetch via `scripts/fetch-*.sh`.
- **There is a display after all.** Playwright's Chromium is cached at
  `~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome` and runs headless
  here, so the UI can be looked at rather than only asserted about:

  ```bash
  # 1. a library to look at, without touching the real ~/Notetaker
  #    (seed meta.json / transcript.md / summary.md / notes.md under $FAKE)
  HOME=$FAKE LD_LIBRARY_PATH=src-tauri/target/debug \
    ./src-tauri/target/debug/notetaker-serve --port 8899 --ui-dir ./dist
  # 2. a browser to look with
  chrome --headless=new --no-sandbox --remote-debugging-port=9333 &
  CHROME_DEVTOOLS_AXI_BROWSER_URL=http://127.0.0.1:9333 \
    chrome-devtools-axi open http://localhost:8899
  ```

- **The Windows machine is right there.** WSL2 interop means `powershell.exe`
  from this shell runs on the host as Mr. Brothers' own user, and `/mnt/c` is
  his drive. That is how the installer was run and the app launched and
  photographed without him touching anything:

  ```bash
  gh run download <run> --name notetaker-windows-installer --dir /mnt/c/Users/georg/Downloads/notetaker
  powershell.exe -NoProfile -Command "Start-Process '<path>\Notetaker_0.1.0_x64-setup.exe' -ArgumentList '/S' -Wait"
  powershell.exe -NoProfile -Command "Get-ChildItem \$env:LOCALAPPDATA\Notetaker"
  ```

  Screenshot the app with `PrintWindow` on `MainWindowHandle` and flag `2`
  (`PW_RENDERFULLCONTENT`, which a WebView2 window needs), **not** a screen
  grab: `SetForegroundWindow` from a background process is refused by Windows,
  so a screen grab catches whatever he actually has open. Files written by the
  installer under `AppData\Local` may not appear through `/mnt/c` straight away
  — ask PowerShell instead of `ls`.

  `chrome-devtools-axi` cannot launch its own Chrome in this WSL2 box (the
  target dies immediately), which is why the browser is started separately and
  attached to. Refs from `snapshot` go stale across a re-render — drive clicks
  with `eval` when a step follows a state change.

## Ground rules
- User data layout (`<home>/Notetaker/Tasks/...`) is a public contract, and is
  **identical on every OS** so a folder copied from a Mac to a PC still opens.
  The SQLite index must always be rebuildable from the files.
- The app dir follows each OS's own convention and holds nothing precious.
- **Nothing ever deletes a recording.** A FLAC encode that cannot be verified by
  decoding it back leaves the WAV; a file too damaged to repair is kept with a
  plain-English note.
- `meta.error` describes a processing *attempt* and clears on retry;
  `meta.capture_note` describes the *audio* and outlives every attempt.
- **Nothing moves a live recording's folder.** Enforced in `runtime.rs`, not the
  UI: a disabled button is not a guarantee.
- **"Idle" means the recording has landed.** Capture reports `Finishing`, never
  `Idle`, until then.
- An error from `CaptureSources::system` means "this platform cannot capture the
  other side of a call", and **meeting mode refuses to start** rather than
  silently recording half a conversation. This is why a Mac currently declines
  meeting mode.
- **The server binds loopback unless LAN access is explicitly turned on**, and
  LAN requires a token on every request including the UI shell — audio included.
  A notetaker that quietly serves meeting transcripts to the coffee-shop wifi is
  a worse failure than any bug in it.
- **The app never rewrites `notes.md`.** The user's own words are one file and
  the AI's are another; the merged document people see is a rendering choice,
  not a stored artifact. A notepad that edits your notes is unusable.
- **The action-item checklist has no storage of its own.** A tick is an edit to
  a checkbox line in `summary.md`, so the list cannot drift from the markdown
  the user can edit by hand.
- The app runs on Mac *and* PC, so **no user-facing string may say "your Mac"**.
- **The window is frameless (`decorations: false`) with a 500px minimum
  width**, pinned by a regression test (`windowControls.test.tsx`) rather
  than left to feel right: while recording, the custom titlebar's
  min-content width is 488px, and with no OS-drawn Close button behind it, a
  window narrower than that clips the mouse's only way to close the app.
- Speech = **Whisper and SenseVoice, routed per segment** by the language
  SenseVoice detects; diarization = sherpa-onnx; summaries = Ollama+Qwen.
  **First run asks which languages the user speaks and downloads accordingly** —
  SenseVoice is 239 MB an English-only user has no use for. Measured on real
  bilingual meetings, not fixtures: `docs/superpowers/specs/bakeoff-result.md`.
  Diarization is verified on real human audio only — synthetic TTS voices
  don't separate.
- Every message a user can hit is written for someone who is not an engineer.

## Verified vs assumed
Retired since B1: the `ioreg` **idle-time** scrape is gone — `MacProbe` now uses
`CGEventSourceSecondsSinceLastEventType`, so it can no longer fail silently and
stop background transcription forever.

Retired 2026-08-05: **system audio**. It had never produced a sample on *any*
platform — Windows' three real recordings all wrote a 0-byte
`audio-system.flac`. ScreenCaptureKit now delivers real audio on a Mac,
measured (see "The Mac day"), which makes macOS the first platform on which
the other side of a call has ever been recorded. Windows loopback is still
unexplained and still owed a run with sound actually playing.

Still assumed:
- **Windows system audio.** See above: 0 bytes on all three real recordings,
  cause unknown. The Mac working does not tell us anything about WASAPI.
- **The macOS microphone permission dialog** — the mic itself is verified
  (`MacBook Pro Microphone`, real samples), but it was granted to a terminal,
  not to `Notetaker.app`. The first launch of the bundle asks again, and a
  *refusal* is still an untravelled path.
- `pmset -g batt` parsing for AC/battery: tested against captured real output,
  never run on a Mac.
- `ollama` — verified against `httpmock` only; the NDJSON field names come from
  knowledge of the API, not observed traffic.
- Meeting detection means "the app is open", not "a call started" — except
  Windows' `CptHost.exe`, which only exists during a Zoom meeting. Browsers are
  deliberately not detected; see `watch/apps.rs`.
- Nothing on Windows beyond the standing caveat that **no audio device has ever
  produced a sample** — the disk-probe fix is confirmed by a real Windows run.

Retired 2026-07-30: "the whole UI, visually" is no longer assumed. Every screen
was driven in a real browser against a real `notetaker-serve` over a real
socket, with real files on disk — the library, the note, the transcript, the
player (click-to-seek moves actual audio, and the current line highlights), the
command palette, the ask panel, settings, both themes, and a 420px phone width.
Two bugs were found and fixed that way and no other way: the first-run card
rendered as two stacked boxes, and the narrow layout squeezed the note into a
155px column.

## CI has run now — what it found (2026-07-30)
The first runs in this project's life, on PR #1. Three rounds.

**State (2026-07-30): macOS ✅ · Linux ✅ · Windows ✅ — the first fully green run
in this project's life.** Every job ran to the end, so for the first time the
**Tauri app crate has been compiled on both Windows and macOS**, and clippy
`-D warnings` passed on all three.

Getting there took two Windows bugs and two CI bugs:
- the disk probe (below) — one bug, eight failing tests, and no recording at all;
- a test written the same day that deleted the SQLite index while the runtime
  still held it open. Unix allows that, Windows refuses (`os error 32`).
  **Windows is a different operating system in the tests too, not only in the
  platform layer.**
macOS had never been compiled before today; it now builds, tests and clippies
clean, as does the Tauri app crate on both platforms.

**Fixed:**
- **macOS: `MicSource` was not `Send`.** `cpal::Stream` is `!Send` on macOS
  while `AudioSource` requires `Send`. The cross-target `cargo check` cannot see
  this — the platform crate alone is fine, and the error only appears where
  *core* uses `MicSource` as a `dyn AudioSource`, and **core cannot be
  cross-checked**. The stream now lives on a worker thread and is never stored,
  so the type is `Send` by construction everywhere. See `platform/src/mic.rs`.
- **All jobs: `pnpm-workspace.yaml` had no `packages` key**, so `pnpm store
  path` failed in setup-node's cache step before any install ran. Also
  `pnpm test --run` — pnpm 9 rejects `--run` as its own unknown option, and the
  package script is already `vitest run`.
- **macOS clippy**: a deprecated CoreGraphics function and an unnecessary
  `unsafe` block, both errors under `-D warnings`.

**A fourth, found 2026-07-30 after the above: the cache made CI lie.** A
documentation-only commit turned all three jobs red. `sherpa-rs` does not build
its native libraries — its build script downloads them, then copies the shared
objects loose into `target/debug/` and `target/debug/deps/`, which is where cargo
finds them when it runs a test. Those copies are **not cargo-tracked artifacts**,
so the cache sweeps them, while cargo still thinks the build script is fresh and
never re-runs it.

It took two passes to fix, because fixing half of it just moved the symptom:
caching the download directory made the *link* succeed and left the test binary
dying at startup with `error while loading shared libraries` (exit 127). The
workflow now caches the downloads **and** runs `cargo clean -p sherpa-rs-sys`,
which forces the one build script to re-copy. Both the failure and the repair
were reproduced locally by deleting the same files by hand.

**A red CI run is not automatically a code problem — check what changed before
believing it.**

**The technique that came out of it: `scripts/check-platforms.sh` runs clippy,
not `cargo check`.** Clippy cross-targets exactly the way `check` does — which
nobody had tried. CI runs clippy `-D warnings` on every OS, so a deprecation in
macOS-only code fails there while `cargo check` locally is perfectly happy. Both
macOS clippy failures reproduce in seconds on Linux with the new script. Run it
before pushing; a CI round trip is about ten minutes.

**The Windows 8 — one bug, found 2026-07-30, and it made the app useless there.**

The earlier note in this place said "seven are the FLAC finalize path" and
guessed at `hound`'s RIFF header. **That was wrong.** Reading the actual failure
output rather than the summary showed two of the eight were not FLAC at all:
*"timed out waiting for the first audio to reach the file"*. All eight are one
cause.

`SysinfoDisk::measure` canonicalized the target path and compared it against a
**raw** mount point. On Windows `canonicalize` returns an extended-length path
(`\\?\C:\Users\...`) while `mount_point()` returns `C:\`, and `Path::starts_with`
compares components — `Prefix(VerbatimDisk)` is not `Prefix(Disk)`. No disk ever
matched, so `free_mb()` was `None`.

That is not merely a probe bug. `Session::disk_trouble` reads `None` as *"could
not read how much storage space is left"* and **stops the recording** — a
deliberate decision ("refusing to record is recoverable, losing a lecture is
not"). So on Windows every recording stopped on its first pump step, before one
sample was written: hence 2 disk-probe failures, 2 "no audio ever arrived", and
4 "the FLAC that was never written is missing". **A shipped Windows build would
have recorded nothing at all.**

Fixed by running both paths through `existing_ancestor`, so like is compared
with like on every OS.

**How it was confirmed without a Windows machine**: breaking the disk match on
Linux produces a failure list *byte-identical* to the Windows CI run — the same
eight tests, in the same order. Worth keeping as a technique: when a platform
you cannot run fails, try to reproduce the failure *set* locally by injecting
the suspected cause. A matching set is much stronger evidence than a plausible
story about one test.

The other thing to take from it: one unreadable disk probe silently disabled the
entire product. The policy is defensible; its blast radius is worth a second
look.

## The scheduler, now wired (2026-07-30)
`Runtime::start_scheduler` used to be reachable only from tests: no production
binary ever built a `SchedulerModels`, so a recording was captured, finalized,
queued — and left there forever. Every layer beneath it worked, which is exactly
why it went unnoticed for two plans. Three things were needed, not one:

- **`Runtime::start_processing`** — resolves the tier, checks the model files,
  loads Whisper and the diarizer, starts the loop. Missing models are a **return
  value (`Processing::ModelsMissing`), not an error**: on a first launch they are
  legitimately absent and the first-run checklist is what fixes it.
- **`Runtime::launch`** — recovery, re-index, and processing in the one call both
  front ends make. `notetaker-serve` previously called *none* of it, so the
  served library's search index was whatever the last run happened to leave
  behind. One shared call because the Tauri shell cannot be compiled here;
  anything it does alone is unverified until CI.
- **`models::ensure_segmentation_unpacked`** — sherpa-onnx ships
  `segmentation-3.0` as a `.tar.bz2`, and nothing unpacked it, so the models
  could download *in full* and the diarizer would still find no `.onnx`.
  **The archive never names the destination**: one member is taken by file name
  and written to a path we chose, so tarball path traversal is not a check that
  has to be right — it is a question that cannot be asked.

The download thread starts processing when it finishes, so a user does not have
to quit and reopen after the checklist goes green.

What is **not** proven: the happy path. Loading a real 500 MB Whisper model is
not a unit test, and no recording has been transcribed end to end by a shipped
binary. The tests pin the decisions around it — before the models arrive, a
second call, and a launch on a machine with nothing downloaded.

## The installer (2026-07-30) — and the fourth time the same three files bit
`tauri build` had **never been run, on any operating system**, so no installer
had ever existed. CI now builds one on every push: the `package-windows` job
produces an unsigned NSIS `.exe` and an `.msi`, uploaded as the
`notetaker-windows-installer` artifact. Getting a build onto a PC is a download,
not an afternoon of installing MSVC.

Building it exposed a real bug before a single byte was packaged. `sherpa-rs`
does not build ONNX Runtime; its build script copies the shared libraries loose
into `target/<profile>/`, which is why a binary run from the build folder works.
Tauri's bundler knows nothing about them, so the installer would have contained
`Notetaker.exe` and nothing else — **it would have installed cleanly and then
refused to start.** The same three files that broke the CI link step, then the
CI load step, then this.

Fixed in `src-tauri/tauri.windows.conf.json` — a Windows-only overlay Tauri
merges over `tauri.conf.json`. Glob patterns rather than explicit file names,
for two reasons: a name that changes on a version bump would not silently drop a
library, and **a Tauri glob that matches nothing is a hard bundler error**, so an
empty installer cannot be built. On Windows the resource root *is* the
executable's directory, which is exactly where a DLL has to be.

That decision paid immediately. The installer carries **five** libraries, and
two of them would have been missed by a hand-written list:
`onnxruntime_providers_shared.dll`, which the `onnxruntime*` glob swept up, and
**`cargs.dll` — which does not exist on Linux at all.** whisper.cpp's CMake
builds `cargs` as a shared library on Windows and links it statically here, so
no amount of care on this machine would have found it. It was caught by running
the same test on Windows.

Three checks, because a config is not a result:
- `core/tests/installer.rs` runs everywhere. It reads the shared libraries the
  build actually produced, translates them to their Windows spelling
  (`libonnxruntime.so` -> `onnxruntime.dll`) and fails if any is not covered by
  a pattern. Confirmed by removing a pattern and watching it fail. On Linux it
  is a fast first filter — **it can only see the libraries Linux produces**,
  which is exactly how `cargs.dll` got through; on Windows it sees the real set.
- CI opens the installer it just built with 7-Zip and asserts every DLL from
  `target/release` is inside it. The config is checked against the artifact.
- CI then reads the import table of `notetaker.exe` and of every bundled DLL
  with `llvm-readobj`, and fails if a binary imports something that is ours to
  ship and is not in the installer. Files being present is not the same question
  as the loader finding what it asks for. The full import list is printed, since
  it is also where the MSVC-runtime dependency would show up.

The import scan then earned its keep on the first run. `notetaker.exe` and the
prebuilt `onnxruntime.dll` both import `MSVCP140` / `VCRUNTIME140` — **the
Microsoft Visual C++ redistributable, which is not part of Windows.** Most PCs
have it because something else installed it; a fresh one does not, and there the
app would have failed to start showing a DLL name. The runtime is now copied
beside the executable ("app-local deployment", which is what it is licensed
for), about 1.5 MB. Nothing else in the chain is unresolved: `notetaker.exe` ->
`sherpa-onnx-c-api.dll` -> `onnxruntime.dll`, all shipped.

The same scan showed `cargs.dll` is imported by nothing at all. It is still
bundled — 139 KB against the risk of being wrong about a library the build
system chose to produce.

Two ordering traps, both found by CI and both worth knowing, because the "a glob
matching nothing is a hard error" property is checked by **`tauri-build` at
compile time**, not only by the bundler:
- a *debug* build of the app crate on Windows therefore cannot succeed, since
  the overlay points at `target/release`. The debug build was dropped from the
  Windows test job; the release build in `package-windows` is the one that
  ships anyway. macOS still builds the app crate in the test job.
- cargo may run the app crate's build script *before* sherpa's, since the two
  are not related by a build dependency — so the globs can be checked before
  the libraries they name exist. `package-windows` builds `notetaker-core`
  first to make the order deterministic rather than a race that had been
  winning.

Green as of run `30584732846`. Every import of every shipped binary now resolves
to a file inside the installer or to Windows itself — printed in full by the
packaging job, so it is a reading rather than a belief.

To get one: open the newest CI run on the branch, `notetaker-windows-installer`
under Artifacts, unzip, run `Notetaker_0.1.0_x64-setup.exe`. Artifacts expire
after 90 days.

```bash
gh run download --name notetaker-windows-installer
```

**Installed and launched on Mr. Brothers' PC, 2026-07-30 — the first time any of
this code has run on real hardware.** Driven from WSL through `powershell.exe`,
which reaches the Windows host: silent install (`/S`, exit 0), then the binary
started and stayed up. All seventeen files landed in
`%LOCALAPPDATA%\Notetaker` beside `notetaker.exe` — the four ONNX/sherpa
libraries, `cargs.dll`, and the nine-file MSVC runtime. The window opens, WebView2
loads, the UI renders in dark theme, and the first-run checklist shows the
languages question. Captured with `PrintWindow` on the window handle rather than
a screen grab, so nothing else on his desktop is in the image.

It is **unsigned**, so SmartScreen warns on a normal double-click ("More info"
-> "Run anyway"); the silent install above did not hit it.

Still unknown, and it is now the *only* thing standing between this and a
working product: **whether a microphone produces a sample.** Nothing beyond the
app's own startup has been exercised on hardware — no recording, no model
download, no transcription.

macOS had the identical bug. **Fixed 2026-08-05** and verified against the built
bundle rather than reasoned about: `bundle.macOS.frameworks` in
`tauri.macos.conf.json` puts the three dylibs in `Contents/Frameworks/`, and
`otool -L` on every shipped binary shows all three `@rpath` loads resolving
inside the bundle. `core/tests/installer.rs` now guards macOS the same way it
guards Windows — and it matters more there, because `frameworks` takes literal
paths and **does not glob**, so the "a glob matching nothing is a hard error"
safety net does not exist. It must name `libonnxruntime.1.17.1.dylib`, the
versioned file; the unversioned `libonnxruntime.dylib` beside it is a symlink
and bundling that instead yields an app that launches and dies on first
transcribe. A test pins exactly that.

## First real use, 2026-07-30 — what a person found in ten minutes
Mr. Brothers installed it and used it. **Capture works on real hardware**: three
recordings, mic audio, meeting mode and in-person mode, FLAC written. That
closes the oldest assumption in this document.

Everything else he found is a real defect, and all of it was invisible to 488
passing tests because none of it is a *failure* — it is the app being silent.

1. **DONE.** "Process now" reported success and transcribed nothing, forever.
   No models were on the machine, so no scheduler existed; `process_now` queued
   the recording and woke nothing. Nothing in the app ever asked the disk — the
   checklist reads `pull_progress()`, which is in-memory, so a restart made a
   set-up app look unset-up and an empty machine look fine. Fixed by
   `Runtime::setup_status` plus a notice that never blocks. **His rule for this
   whole area: "dont force it — i just want the app to be like okay fine but
   just so u know it wont work."**
2. **DONE. Playback, on every recording, processed or not.** The player was
   never broken; it sat below `TranscriptPanel`'s no-segments guard, so raw
   audio had no route to it. `NoteView` now owns one shared player, opened from
   its Listen control or the Transcript tab. `audio_tracks` also rejects
   header-only WAVs while retaining genuinely quiet recordings, so a silent
   system track is no longer offered as playback.
3. **Settings — all four are wrong**, his answer: nothing is pre-filled with
   what the app already chose; no microphone picker; no model status
   (downloaded, size, re-download, tier); storage location neither visible nor
   changeable.
4. **Ollama: detect an existing install** rather than pushing a download, and
   let the user pick which of *their* models writes summaries.
5. **Processing stays automatic** (idle-gated) with the manual button always
   available. Already the design; only the honesty was missing.

Two capture bugs found in his files, not yet chased:
- `audio-system.flac` is **0 bytes on all three** recordings. WASAPI loopback
  produced nothing. Unknown whether it is broken or he simply had no sound
  playing — do not assume the second.
- Two of the three kept `audio-mic.wav` **next to** the `.flac`. Per the ground
  rules that means FLAC verification did not confirm — but `meta.error` is
  `null`, so whatever happened was never reported. The silence is the bug
  regardless of the cause.

## The Mac day, 2026-08-05 — and the bug only hardware could find
The constraint this whole document was written under is gone. Every line of
this project was authored on a Linux box that could not compile macOS code, let
alone run it; the Mac now builds all four crates natively in about 30 seconds
and runs the tests in two.

**What was actually missing, in the order it mattered:**

- **`NSMicrophoneUsageDescription` did not exist anywhere in the repo.** There
  was no `Info.plist` and no entitlements file at all. This is not a nicety: a
  macOS process that touches the microphone without that key is **killed by the
  OS**, not denied. The app would have died on the first press of Record with
  nothing to explain it. Now in `src-tauri/Info.plist`, which Tauri merges.
- **Metal was never compiled in.** `whisper-rs` is `default = []` and no feature
  was set in any manifest, while `detect_tier` puts every Apple Silicon machine
  with ≥16 GB on `AppleSiliconBig` — a tier whose stated justification is "the
  large model on the GPU". The tier was promising something the build did not
  contain. Confirmed fixed by reading the binary, not the config: `Metal.framework`
  and `MetalKit.framework` are linked, and a real run logs
  `GPU name: Apple M5 Pro` with `Metal total size = 1623.92 MB` — the whole
  `large-v3-turbo` resident on the GPU.
- **No full Xcode needed.** Command Line Tools carry the ScreenCaptureKit SDK,
  and whisper.cpp embeds its Metal shaders as *source* compiled at runtime
  (`ggml-metal-embed`), so the absent `xcrun metal` never comes up. One thing is
  left on the table: `ggml_metal_library_init_from_source: error compiling
  source` → *"the tensor API is not supported in this environment - disabling"*.
  Installing Xcode may unlock the M5's tensor path. Untested, and worth a try.

**The bug that justifies all of this.** ScreenCaptureKit started cleanly, called
the delegate at the correct rate with `type=1` (audio), and delivered **zero
samples, forever**. `CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer`
returned `kCMSampleBufferError_ArrayTooSmall` (-12737) on every buffer while
being handed a list *four times larger* than it asked for — 136 bytes provided
against 40 needed.

The name is a lie. The size passed must describe a list of **exactly** the
number of buffers the sample holds, not merely one big enough to hold it. So
ask first, then fill: one call with a null list to learn the size, a second with
that size. Nothing about this is visible to a compiler, a unit test, or CI. It
compiles, it starts, it calls back, and it records silence — which is precisely
the outcome the ground rules call worse than any crash.

**How that gets caught now.** Two examples exist because no unit test can open a
device and no CI runner has one:

```bash
cargo run -p notetaker-platform --example system-audio   # play something first
cargo run -p notetaker-platform --example microphone     # then say something
```

Both were run, and both passed for the first time in this project's life:

| | samples in 5 s | peak | dropped |
|---|---|---|---|
| system audio | 79,146 (~4.9 s @ 16 kHz) | 0.2966 | 0 |
| microphone | 79,487 (~5.0 s @ 16 kHz) | 0.0260 | 0 |

They print sample counts *and* peak amplitude on purpose: "opened and delivered
nothing" and "delivered nothing but silence" are different bugs with the same
appearance, and the second is the one that ships.

**Two things worth keeping:**
- `rustc-link-arg-bins` only reaches binaries of the package that emits it. The
  app crate's `build.rs` did nothing for `notetaker-serve`, which then died with
  `no LC_RPATH's found` — macOS has no system fallback the way Linux does. The
  server crate has its own `build.rs` now.
- `notetaker-serve` does not bind its socket until `Runtime::launch` finishes,
  and loading a 1.6 GB model onto Metal takes ~20 s. It looks dead for that
  whole time.

## The signature, and the silence (2026-08-05, second pass)
First real use on the Mac reproduced the Windows symptom exactly: an
eleven-minute Zoom recording with a 5 MB `audio-mic.flac` beside a **0-byte**
`audio-system.wav`, `status: "failed"`, and an error blaming a "damaged" file.
The mic was perfect. Two separate bugs produced it.

**Tauri never signed the bundle.** What shipped was the linker's automatic
ad-hoc signature — `Identifier=notetaker-0764321f84a1de0c` against a
`CFBundleIdentifier` of `com.georgebrothers.notetaker`, `Info.plist=not bound`,
and `codesign --verify` failing with *"code has no resources but signature
indicates they must be present"*. **TCC identifies an app by its code
signature**, so a Screen Recording grant had nothing stable to attach to and did
not survive a rebuild. `signingIdentity: "-"` fixes it; the bundle now verifies
valid and satisfies its Designated Requirement.

That turns on the **hardened runtime**, which is why `entitlements.plist` now
exists. Under it macOS refuses the microphone without
`com.apple.security.device.audio-input`, and refuses to load the unsigned ONNX
and sherpa dylibs out of `Contents/Frameworks` without
`com.apple.security.cs.disable-library-validation`. Neither failure announces
itself.

**The second bug is the one worth remembering.** *macOS does not report a
missing Screen Recording grant.* `SCShareableContent` returns displays,
`startCapture` completes without error, and the delegate is simply never called
— so the code believed it was recording. Every design in this file had assumed a
refusal would arrive as an error.

The discriminator is measured, not reasoned:

| Screen Recording | nothing playing | audio playing |
|---|---|---|
| granted | 78,463 samples, peak **0.0000** | ~79,000 samples, peak 0.2966 |
| not granted | **nothing at all** | **nothing at all** |

A silent room still streams buffers at full rate; a missing grant streams
nothing. So `start()` waits three seconds for a first *buffer* (not a non-zero
sample — that would fail on a quiet room) and treats its absence as a refusal.
Meeting mode then declines with the plain-English message, which is what the
`CaptureSources::system` contract always intended.

**The wider lesson: a permission that is missing usually looks like data that is
absent, not like an error.** Anywhere the app depends on a grant it did not
verify, assume silence rather than a failure, and check for arrival.

**Resolved (2026-08-05, third pass): ad-hoc was not enough.** `signingIdentity:
"-"` makes the signature *valid* but pins the app's identity to the build's
cdhash — so every rebuild orphaned the Screen Recording grant while the
Settings checkbox stayed on, which is how Mr. Brothers granted it "25 times"
and it never held. The fix is a stable identity: a self-signed certificate
**"Notetaker Local Signing"** in `~/Library/Keychains/notetaker-sign.keychain-db`
(keychain password stored as the `notetaker-sign-keychain` generic password in
the login keychain; the CI-style separate keychain exists because the login
keychain's own password could not be matched for `set-key-partition-list`).
`tauri.macos.conf.json` signs with it, so the designated requirement is
`certificate leaf = H"cc31effe…"` — stable across every rebuild. After changing
a signing identity, clear the orphaned grant once:
`tccutil reset ScreenCapture com.georgebrothers.notetaker`. Verified by Mr.
Brothers pressing Record: mic audible on playback, and the app's log shows
`system audio: first buffer read, 960 samples` — in-app capture, proven.

**Also found: the two front ends use different app directories.** The Tauri
shell calls `app.path().app_data_dir()` →
`~/Library/Application Support/com.georgebrothers.notetaker`, while
`notetaker-serve` uses `paths::default_app_dir()` →
`~/Library/Application Support/Notetaker`. Different `index.sqlite`, different
`settings.json`, different logs — despite `open_runtime`'s comment promising the
two transports are built identically. Not yet fixed; it cost an hour of reading
the wrong log file.

## Next
1. **Windows system audio is still 0 bytes.** The Mac working tells us nothing
   about WASAPI. Re-run a Windows recording with sound definitely playing, and
   if it is still empty, port the diagnostic-example approach to Windows — it is
   what found the Mac bug in one run.
2. **The two front ends disagree on the app directory.** The Tauri shell
   uses `app.path().app_data_dir()`, `notetaker-serve` uses
   `paths::default_app_dir()`. Two indexes, two settings files, two logs, on
   every OS. Pick one — `paths::` is the contract — and migrate.
3. **Non-speech leaks into transcripts, from both engines.** Whisper writes
   `[MUSIC PLAYING]` and `[BLANK_AUDIO]`; SenseVoice hallucinates a short
   interjection (`あ。`) onto the same silence. Both were seen in real audio on
   2026-07-30. Whisper's markers are trivially filterable; SenseVoice's are not
   distinguishable from a real short utterance, so the honest fix is probably a
   VAD gate before transcription rather than a text filter after it.
4. **Whisper pads every call to a 30-second window.** A recording with many
   short diarization segments therefore costs 30 s of compute per segment
   regardless of length — 70 segments of a 4-minute recording took over ten
   minutes. This predates routing and is the single biggest processing cost in
   the app. Batching adjacent same-speaker spans would cut it directly.
5. **On the PC — the one thing left.** It is installed and it runs. **No audio
   device has ever produced a sample through this code**, on any platform, and
   nothing else has been exercised on hardware either: the model download, a
   recording, a transcription. Hit Record and find out. This can be driven from
   here through `powershell.exe` (see "Build environment") rather than handed
   to him as a chore.
6. **On the Mac — a full run through the app itself.** The pieces are all
   verified individually; what has *not* happened is one recording taken end to
   end inside `Notetaker.app` — permission dialogs accepted as the bundle rather
   than as a terminal, a meeting recorded, transcribed on Metal, and summarized
   by Ollama (`qwen3:8b` is installed and serving). That needs a person to click
   Allow twice, and is the last thing standing between this and a working Mac
   product. Re-run the bake-off against `large-v3-turbo` while there.
7. **A macOS packaging job in CI**, mirroring `package-windows` — build the
   `.app`/`.dmg` and assert every `@rpath` load resolves inside the bundle, the
   way the Windows job reads import tables. The `.app` is currently only ever
   built by hand.
8. **`tauri build` cannot finish unattended.** `createUpdaterArtifacts` is on
   and the signing step fails with *"A public key has been found, but no private
   key"* — `TAURI_SIGNING_PRIVATE_KEY` is unset. The `.app` and `.dmg` are
   produced before it fails, so this is not fatal to a local build, but no
   release can be cut until the key is available.
9. **82 frontend tests fail on the UI-overhaul branch**, in three files, all
   from one cause: `shell` is undefined at `capture.test.tsx:475`, so
   `shell.handlers.clear()` throws in `beforeEach`. Confirmed pre-existing by
   running them on the untouched branch — unrelated to any Mac work. It makes
   `pnpm test` useless as a gate until fixed.
10. **Signing.** The installer is unsigned, so SmartScreen warns on first launch.
   Fine for one user; a wall for anyone else. Needs a certificate, i.e. money.
11. **Windows truth pass for the UI overhaul** — nothing about the frameless
   window, the tray, or the global hotkeys has been verified on real
   hardware yet; CI compiles and bundles the app but never opens a window.
   Install the CI build, then check: tray icon states (idle/recording/
   paused) at 100% and 150% DPI, tray menu labels and Quit, close-to-tray +
   first-time "Still running" note, both global hotkeys with the window
   closed, titlebar drag / double-click / edge-snap, autostart after a
   reboot, mic picker lists real devices, `PrintWindow` screenshots of dark
   + light against the pitch.
12. **Light-mode contrast on the first-run gradient title.** The "Getting
   started" heading (15px/600, gradient `background-clip: text`) measures
   4.41:1 at its cyan end — just under the 4.5:1 WCAG AA minimum for that
   size and weight. Dark mode is fine.
13. **The playhead thumb's glow is WebKit-only.**
   `input[type="range"]::-webkit-slider-thumb` carries the accent glow in
   `panels.css`; there is no `::-moz-range-thumb` rule, so Firefox users of
   the served web UI see a playhead with no glow.
14. **No `forced-colors` fallback on gradient text.** The same first-run
    title uses `color: transparent` + `background-clip: text` with nothing
    behind it for Windows High Contrast Mode — it could render invisible
    there.
