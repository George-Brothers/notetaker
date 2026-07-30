# MAP — Notetaker (personal)

A fully local notetaker for Mr. Brothers, on **Mac and PC**, with the same UI
also **served to a browser** so the library can be read from a phone. High-quality
dual-track recording now, idle-time local transcription (EN/ZH, Speaker 1/2/3)
and summarization later, organized by tasks. No cloud, ever.

## State
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
  Remaining: **macOS system audio** (ScreenCaptureKit).
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
    `macos/` (CoreGraphics idle; ScreenCaptureKit **not yet written**).
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
  - Tailwind v4 + Radix + Lucide + cmdk + a self-hosted Inter. Every colour is
    a token in `styles/theme.css`; **no component may hardcode one**, or the
    dark theme silently loses that element. `styles/panels.css` styles Settings
    and the first-run checklist, which keep their tested markup.
  - `components/ui.tsx` holds the shared primitives.
- `fixtures/` — `bilingual.wav`, `diarization-check.wav`, reference transcript.

## How this is verified — read this before trusting anything
Nothing here can *run* macOS or Windows code. The thing that makes writing it
anyway defensible is one property: **`cargo check` does not link**, so a crate
with pure-Rust dependencies type-checks against a foreign target with no
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
| `windows/`, `macos/`, `mic` | **Compile-verified** for the real target. Never run |
| `capture::platform`, `power::probe` per-OS arms | **CI only.** Never compiled on Linux |
| The Tauri app crate | **CI only.** Never compiles on Linux |
| `notetaker-server` + `dispatch` | **Fully verified here**, including a real binary over a real socket |
| The whole UI, visually | **Seen and screenshotted** (2026-07-30) — real binary, real files, real audio, in Chrome. See below |
| The scheduler wiring | **Decisions tested; the happy path is not.** A real model load is not a unit test — see "The scheduler, now wired" |
| Speech routing | **Measured on real bilingual meetings** (2026-07-30), both models loaded, against a Whisper-only baseline. See `specs/bakeoff-result.md` |
| The Windows installer | **Built and its contents checked** by CI. Nobody has run it — see "The installer" |

The cross-check was itself confirmed with a negative control: a deliberate type
error in `windows/power.rs` *is* caught.

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

Still assumed:
- **Every capture path.** No audio device has ever produced a sample through
  this code. CI runners have none either, so CI will not close this — it needs a
  human at a real machine.
- **macOS Screen Recording permission** — never requested, granted or refused.
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

Still unknown: whether the installer installs, and whether the installed app
starts. Nothing has run it. It is also **unsigned**, so SmartScreen will warn on
first launch ("More info" -> "Run anyway").

macOS has the identical bug and is not fixed: `.app` resources land in
`Contents/Resources/` while the binary is in `Contents/MacOS/`, so the dylibs
need `Contents/Frameworks` and an rpath instead. Left alone rather than guessed
at, because it cannot be verified from here and the Mac work is blocked anyway.

## Next
1. **macOS system audio** — ScreenCaptureKit. The full design and the reason it
   was not written blind are in `platform/src/macos/speaker.rs`. Everything below
   it (ring, downmix, resample) is already shared and tested.
2. **Non-speech leaks into transcripts, from both engines.** Whisper writes
   `[MUSIC PLAYING]` and `[BLANK_AUDIO]`; SenseVoice hallucinates a short
   interjection (`あ。`) onto the same silence. Both were seen in real audio on
   2026-07-30. Whisper's markers are trivially filterable; SenseVoice's are not
   distinguishable from a real short utterance, so the honest fix is probably a
   VAD gate before transcription rather than a text filter after it.
3. **Whisper pads every call to a 30-second window.** A recording with many
   short diarization segments therefore costs 30 s of compute per segment
   regardless of length — 70 segments of a 4-minute recording took over ten
   minutes. This predates routing and is the single biggest processing cost in
   the app. Batching adjacent same-speaker spans would cut it directly.
4. **On the PC**: download the installer from the CI run, install it, and find
   out whether it records. **No audio device has ever produced a sample through
   this code**, on any platform — that one sitting closes the largest remaining
   unknown in the project. Then the first-run model download and one real
   bilingual call end to end.
5. **On the Mac**: bundle the dylibs (above), Metal build and tier detection,
   Screen Recording permission, and re-run the bake-off against
   `large-v3-turbo`. Blocked behind ScreenCaptureKit either way.
6. **Signing.** The installer is unsigned, so SmartScreen warns on first launch.
   Fine for one user; a wall for anyone else. Needs a certificate, i.e. money.
