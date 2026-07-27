# Plan B — Capture, watching, power, and the shell

Written 2026-07-27. Follows Plan A (portable core, complete). Spec:
`docs/superpowers/specs/2026-07-23-notetaker-design.md` §§4.1, 4.2, 4.6.

## The split that makes this buildable today

Plan B was registered as "when the Mac arrives." That was too coarse. Only
three things genuinely need the Mac:

1. **System-audio capture** (ScreenCaptureKit) — no equivalent on WSL2.
2. **Compiling the Tauri app crate** — needs webkit/gtk (and pkg-config, which
   we have no sudo to install; `cargo check -p notetaker` dies in
   `libdbus-sys`). None of that exists on macOS, where the crate builds fine.
3. **Permissions, DMG, real-hardware e2e.**

Everything else is portable Rust or React. So Plan B is two halves:

- **B1 — buildable and testable now (this plan, tasks 1–8).**
- **B2 — the Mac day (registered at the bottom, ~2026-07-30).**

The governing design rule stays Plan A's: **all logic lives in
`notetaker-core`, which compiles and tests here.** The app crate is mechanical
glue. Every Mac-only surface is a trait with a fake, so the Mac day is
"implement two traits and compile," not "write the app."

---

## Task 1 — Capture session engine

**Files:** create `src-tauri/core/src/capture/session.rs`,
`src-tauri/core/src/capture/track.rs`, `src-tauri/core/src/capture/source.rs`.
Modify `src-tauri/core/src/capture/mod.rs` (types pre-wired).

**Interfaces:**
- Consumes `Store::create_recording`, the `AudioSource` trait (pre-wired).
- Produces `Session` — the state machine behind Start/Pause/Resume/Stop.
  Writes `audio-mic.wav` and (meeting mode) `audio-system.wav` into the
  recording dir; on stop sets `meta.duration_s` and status `Recorded`.

**Done criteria:**
- `TrackWriter` appends f32 frames to a 16 kHz mono WAV, flushing at least
  every `FLUSH_INTERVAL` so a crash loses ≤ that much audio.
- `Session::start(mode)` creates the recording dir + `meta.json` *before* the
  first sample lands, so a crash mid-recording still leaves a recoverable dir.
- Pause stops consuming frames without closing the file; resume continues the
  same file (no gap markers, no second file).
- Disk guard: below `MIN_FREE_MB` free, the session auto-stops, finalizes what
  it has, and sets `meta.error` to a sentence a non-engineer can act on.
- `Session::status()` returns `CaptureStatus` (state, elapsed, per-track peak
  level, disk free) for the UI's meters.
- `meta.duration_s` is real after stop (kills carry-over M5).

**Tests:** drive the whole thing with `FakeSource` — start/pause/resume/stop
produces one WAV whose sample count matches elapsed minus paused; a source
that errors mid-stream still finalizes a playable file; the disk guard trips
on an injected low-free-space probe.

**Commit:** `feat: dual-track capture session with pause, flush, and disk guard`

---

## Task 2 — Crash recovery and FLAC finalize

**Files:** create `src-tauri/core/src/capture/recover.rs`,
`src-tauri/core/src/capture/flac.rs`.

**Interfaces:** consumes Task 1's on-disk WAVs; produces `recover_orphans(store)`
run at app start, and `finalize_to_flac(path)` called after stop.

**Done criteria:**
- `repair_wav_header(path)` fixes the RIFF/data lengths of a WAV whose writer
  died mid-recording (header says 0 bytes, file has hours). Repaired file
  loads through the existing `pipeline::audio::load_mono_16k`.
- `finalize_to_flac` encodes 16 kHz mono WAV → FLAC via `flacenc`, verifies the
  FLAC decodes to the same sample count, and only then deletes the WAV
  (unless `settings.keep_wav`). Never deletes on a failed encode.
- `recover_orphans` scans for recordings stuck in a mid-capture state, repairs
  and finalizes them, and marks them `Recorded` so they enter the queue.

**Tests:** truncate a known WAV's header → repair → sample count matches the
original; WAV→FLAC→decode round-trips within tolerance; a corrupt WAV that
cannot be repaired is left on disk with `meta.error` set, never silently
deleted.

**Commit:** `feat: wav crash repair and lossless flac finalize`

---

## Task 3 — Meeting watcher

**Files:** create `src-tauri/core/src/watch/apps.rs`,
`src-tauri/core/src/watch/watcher.rs`.

**Interfaces:** consumes `sysinfo` behind the pre-wired `ProcessSource` trait;
produces `Watcher::poll() -> Vec<MeetingEvent>` and the per-app policy read
from `Settings::auto_record`.

**Done criteria:**
- Known-app table covering Zoom, Teams, Meet/Chrome, Slack, Webex, Discord,
  FaceTime — matched on process name, with the display name the UI shows.
- Debounce: an app must be seen for `CONFIRM_POLLS` consecutive polls before
  `Started` fires, and missing for `CONFIRM_POLLS` before `Ended`. A process
  that flaps produces no events.
- Policy resolution: `Ask` (default) → event reaches the UI prompt; `Always` →
  event carries `auto_start: true`; `Never` → no event at all.
- Never fires twice for one continuous meeting.

**Tests:** a scripted `FakeProcessSource` timeline (absent → present ×N →
absent) asserts exactly one Started and one Ended; flapping produces none;
each policy produces the documented outcome.

**Commit:** `feat: meeting watcher with debounce and per-app auto-record policy`

---

## Task 4 — Real idle and power gating

**Files:** create `src-tauri/core/src/power/mod.rs`,
`src-tauri/core/src/power/probe.rs`.

**Interfaces:** produces `PowerPolicy` implementing Plan A's `IdleSource`
trait, replacing `AlwaysIdle` in production. macOS probe shells out to
`ioreg`/`pmset` behind a `SystemProbe` trait.

**Done criteria:**
- `PowerPolicy::ok_to_run` is true only when idle ≥ `min_idle_secs` AND
  (`!require_ac` OR on AC) AND battery ≥ floor — all from `Settings`.
- `process_when_idle == false` means "run whenever," not "never run."
- The macOS probe is `#[cfg(target_os = "macos")]`; a probe failure degrades to
  "not idle" (never processes at a bad time because a shell-out broke).

**Tests:** the policy is pure over a `FakeProbe` — table test across
idle/AC/battery combinations including every boundary.

**Commit:** `feat: real idle and power gating for background processing`

---

## Task 5 — Ollama manager

**Files:** create `src-tauri/core/src/ollama/mod.rs`.

**Interfaces:** consumes `ureq`; produces `status()`, `pull(model, on_progress)`,
`ensure_model(model)`.

**Done criteria:**
- `status()` reports installed / running / models present / whether the
  configured model is there, and never blocks longer than a short timeout.
- `pull` streams Ollama's NDJSON progress and reports percent to a callback,
  so the UI shows a real bar rather than a spinner.
- `ensure_model` is idempotent: a model already present pulls nothing.
- A missing Ollama produces install *instructions* (the actual install is the
  user's click on the Mac), never a crash.

**Tests:** `httpmock` serves a canned NDJSON pull stream → progress callbacks
are monotonic and end at 100; a 404 model → actionable error; a dead port →
`status()` says not-running rather than erroring.

**Commit:** `feat: ollama status, model pull with progress, and ensure-model`

---

## Task 6 — Runtime: the app crate's whole surface

**Files:** create `src-tauri/core/src/runtime.rs`.

This is the task that de-risks the Mac day. The app crate cannot compile here,
so it must contain as close to zero logic as possible.

**Interfaces:** produces `Runtime` owning `Store`, `Index`, `Queue`, `Session`,
`Watcher`, settings path, and the scheduler `Waker` — with one method per
`#[tauri::command]`, each returning a serializable type.

**Done criteria:**
- Every existing `api::*` function plus the new capture/watch/ollama commands
  is reachable as a `Runtime` method. The app crate's job shrinks to
  `#[tauri::command] fn x(state: State<Runtime>) -> Result<T, String>`.
- **Carry-over I3:** the scheduler's success path calls
  `index.upsert(&rec, transcript, summary)`, so a just-processed recording is
  searchable immediately. Regression test: process → search → hit, with no
  rebuild in between.
- **Carry-over I4:** `Runtime::start_scheduler()` spawns the thread, builds the
  `Waker`, and `process_now` wakes it.
- **Carry-over M4:** a documented, tested list of command names + argument
  names in camelCase, checked against `src/lib/ipc.ts` by a test that fails if
  the two drift.

**Tests:** an end-to-end runtime test on a temp dir — start capture (fake
source) → stop → queue → scheduler tick → ready → search finds it.

**Commit:** `feat: runtime facade, index-on-completion, scheduler thread, ipc contract test`

---

## Task 7 — UI: recording controls and the meeting prompt

**Files:** create `src/components/RecordBar.tsx`,
`src/components/MeetingPrompt.tsx`, `src/hooks/useCapture.ts`. Modify
`src/App.tsx`. Test `src/components/__tests__/capture.test.tsx`.

**Done criteria:**
- Record bar: mode picker (Meeting / In-person), Start, Pause/Resume, Stop,
  elapsed timer, two level meters, disk-space warning.
- In-person mode visibly records one track; meeting mode two. The UI never
  offers system audio in in-person mode.
- `MeetingPrompt`: "Zoom started — record this?" with Record / Not now /
  Always for Zoom / Never for Zoom, the last two writing `Settings.autoRecord`.
- Keyboard reachable; no layout shift when the timer ticks.

**Tests:** vitest with mocked `api` — start calls `startCapture` with the
picked mode; pause/resume toggles label and calls the right command; the
prompt's four buttons each call what they claim; "Always" persists policy.

**Commit:** `feat: recording controls and meeting-detected prompt`

---

## Task 8 — UI: settings and first-run

**Files:** create `src/components/Settings.tsx`, `src/components/FirstRun.tsx`.
Modify `src/App.tsx`. Test `src/components/__tests__/settings.test.tsx`.

**Done criteria:**
- Settings: storage location, model tier (+ detected tier shown), download
  progress per model, Ollama status with a Pull button and progress, auto-record
  policy per known app, processing rules (idle minutes, require AC), keep-WAV.
- First-run: an ordered checklist — permissions (Mac), download models, install
  Ollama + pull the model — each item live-reflecting real status, with the app
  usable in a degraded state rather than blocking.
- Every control writes through `api.setSettings` and reflects the round-trip.

**Tests:** vitest with mocked `api` — each control persists the documented
field; the checklist marks an item done when its status call reports ready; a
failed pull surfaces the error text.

**Commit:** `feat: settings screen and first-run checklist`

---

## B2 — the Mac day (~2026-07-30)

Registered, not built here. Each item is small *because* B1 left a trait:

1. `capture::source::MacSystemSource` — ScreenCaptureKit system audio,
   implementing the same `AudioSource` trait `FakeSource` implements.
2. `capture::source::MacMicSource` — cpal, already a macOS-only dependency.
3. `power::probe::MacProbe` — real `ioreg`/`pmset` values.
4. Compile the app crate: the `#[tauri::command]` wrappers over `Runtime`,
   menu-bar window, tray, event emission, permission prompts.
5. Metal builds of whisper/sherpa + tier auto-detect on real hardware.
6. Screenshot verification of every UI surface (no display on WSL2).
7. DMG packaging and signing.
8. Re-run the bake-off vs `large-v3-turbo`; confirm or revise the SenseVoice
   default.
9. End-to-end: record a real bilingual call → idle-process → verified
   transcript + summary.

## Risks

- **`flacenc` is new to this repo.** Task 2 verifies every encode by decoding it
  back before deleting the WAV, so the worst case is wasted space, not lost
  audio. If the crate disappoints, WAV-only ships and FLAC moves to B2.
- **ScreenCaptureKit is the one real unknown.** It is isolated behind one trait
  with a working fake, so it cannot block anything else in the plan.
- **No display here.** Every UI task's acceptance is vitest behavior, with the
  visual beat explicitly deferred to B2 item 6.
