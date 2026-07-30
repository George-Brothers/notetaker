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
  meeting detection), the core adapter, the **served web UI**, and CI.
  Remaining: **macOS system audio** (ScreenCaptureKit) and the **Tauri shell**.
  Plan and decisions: `docs/superpowers/plans/2026-07-29-cross-platform.md`.
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
- **`.` (app crate)** — the Tauri shell. **Still the generated scaffold.** Does
  not build on Linux (`libdbus-sys` needs pkg-config we have no sudo for).
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
- Speech = SenseVoice (default) / Whisper (fallback); diarization = sherpa-onnx;
  summaries = Ollama+Qwen. Diarization is verified on real human audio only —
  synthetic TTS voices don't separate.
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
- **The CI workflow has never run**, and **this repo has no git remote at
  all** — so nothing has ever been pushed and no PR exists. Creating a remote
  publishes a private notetaker's source, which is Mr. Brothers' call.

Retired 2026-07-30: "the whole UI, visually" is no longer assumed. Every screen
was driven in a real browser against a real `notetaker-serve` over a real
socket, with real files on disk — the library, the note, the transcript, the
player (click-to-seek moves actual audio, and the current line highlights), the
command palette, the ask panel, settings, both themes, and a 420px phone width.
Two bugs were found and fixed that way and no other way: the first-run card
rendered as two stacked boxes, and the narrow layout squeezed the note into a
155px column.

## Next
1. **Tauri shell** — the app crate is still the generated scaffold. Now much
   smaller than it was: `core::dispatch` already does the work, so the ~23
   wrappers collapse into thin forwarding. Plus capabilities and per-OS bundling
   (DMG / MSI+NSIS).
2. **macOS system audio** — ScreenCaptureKit. The full design and the reason it
   was not written blind are in `platform/src/macos/speaker.rs`. Everything below
   it (ring, downmix, resample) is already shared and tested.
3. **On the hardware**: run CI, then real capture on both machines, Metal build
   and tier detection, permissions, the screenshot pass, re-run the bake-off, and
   one real bilingual call end to end.
