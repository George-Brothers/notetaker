# MAP — Notetaker (personal)

Fully local macOS notetaker for Mr. Brothers: high-quality dual-track
recording now, idle-time local transcription (EN/ZH, Speaker 1/2/3) and
summarization later, organized by tasks. No cloud.

## State
- **Plan A (portable core): COMPLETE** (2026-07-23) — storage, index, queue,
  pipeline, models, UI library view.
- **Plan B1 (everything not Mac-locked): COMPLETE** on branch
  `plan-b-capture` (2026-07-27). Capture engine, crash recovery + FLAC,
  meeting watcher, idle/power gating, Ollama manager, the `Runtime` facade,
  and the remaining UI. **217 Rust + 49 frontend tests green, clippy clean.**
- **Plan B2 (the Mac day): not started**, waiting on the hardware (~2026-07-30).
  Scope and a precise checklist at the bottom of the Plan B doc — it is short
  by design, because B1 left a trait with a working fake behind every
  platform-bound surface.
- **Spec:** `docs/superpowers/specs/2026-07-23-notetaker-design.md`.
- **Plan A:** `docs/superpowers/plans/2026-07-23-notetaker-v1-core.md`
  (read its "Plan amendment" section — WSL2 constraints reshaped several tasks).
- **Plan B:** `docs/superpowers/plans/2026-07-27-notetaker-v2-capture.md`.
- **Bake-off decision:** `docs/superpowers/specs/bakeoff-result.md` —
  SenseVoice beats Whisper-tiny on Chinese; it is the default speech engine.
  Re-run against `large-v3-turbo` on the Mac before the call is final.

## Layout (as built)
- `src-tauri/core/` — crate `notetaker-core`: **all** portable logic.
  - `capture/` — `session` (state machine), `track` (incremental WAV),
    `flac` (verified lossless finalize), `recover` (crash repair),
    `source` (the `AudioSource` seam + fakes).
  - `storage` (files-first layout), `index` (SQLite FTS5, CJK-segmented),
    `queue` (crash-safe, 3× retry), `pipeline/` (audio, transcribe, diarize,
    merge, llm, summarize, suggest, run), `models` (registry + downloader),
    `watch/` (meeting detection), `power/` (idle + AC gating),
    `ollama/` (status, pull, ensure), `scheduler`, `api`, `runtime`.
  - Tested via `cargo test -p notetaker-core` from `src-tauri/`.
- `src-tauri/` (app crate) — thin Tauri shell. Does NOT compile on Linux
  (`libdbus-sys` needs pkg-config we have no sudo for; none of that exists on
  macOS). Written and compiled on the Mac in B2.
- `src/` — React/TS UI: library, record bar, meeting prompt, settings,
  first-run, search, speaker + recording rename. `src/lib/ipc.ts` is the
  Rust↔UI contract.
- `fixtures/` — `bilingual.wav`, `diarization-check.wav`, reference transcript.

## Build environment (WSL2, hard-won)
- `cargo test -p notetaker-core` from `src-tauri/` is the only Rust check here.
  Needs `PATH=$HOME/.cargo/bin` and `LIBCLANG_PATH=$HOME/.local/lib/libclang`.
- `pnpm test --run` runs the UI tests; **`pnpm build` is the only typecheck** —
  vitest does not typecheck, so a contract change can pass the tests and still
  be broken. Run both.
- Run clippy with `--all-targets`; without it, test code is never linted.
- `models/` is gitignored; fetch via `scripts/fetch-*.sh`.
- **No display.** Every UI acceptance here is test behaviour; the visual pass
  is B2 item 6 and has never been done.

## Ground rules
- User data layout (`~/Notetaker/Tasks/...`) is a public contract; the SQLite
  index must always be rebuildable from the files.
- **Nothing ever deletes a recording.** A FLAC encode that cannot be verified
  by decoding it back leaves the WAV; a file too damaged to repair is kept with
  a plain-English note. Wasted disk is recoverable, a lecture is not.
- `meta.error` describes a processing *attempt* and clears on retry;
  `meta.capture_note` describes the *audio* and outlives every attempt.
- Speech = SenseVoice (default) / Whisper (fallback); diarization = sherpa-onnx;
  summaries = Ollama+Qwen. Diarization is verified on real human audio only —
  synthetic TTS voices don't separate.
- Every message a user can hit is written for someone who is not an engineer.

## Verified vs assumed — read before the Mac day
Built and tested here, but **never executed on macOS**:
- `power::probe::MacProbe` — the `ioreg`/`pmset` *parsers* are tested against
  captured real output; that those commands emit that shape on the actual
  machine is not. Fails safe (unreadable → "not idle" → processing waits), but
  it fails *silently*, so check it explicitly.
- `ollama` — verified against `httpmock` only. No real Ollama was contacted, so
  the NDJSON field names come from knowledge of the API, not observed traffic.
- Meeting detection for Slack/Teams/Discord means "the app is open", not "a call
  started". Browsers are deliberately not detected at all — see the reasoning in
  `watch/apps.rs`, and B2's window-title work.
- The whole UI, visually. Behaviour is tested; appearance is not.

## Next (B2, on the Mac)
`CaptureSources` (cpal mic + ScreenCaptureKit system audio) and `MacProbe` are
two trait impls; the app crate is ~23 mechanical `#[tauri::command]` wrappers
over `runtime::COMMANDS`, whose argument names a test already pins against
`ipc.ts`. Then: Metal builds + tier detect on real hardware, screenshot pass,
permissions, DMG, re-run the bake-off, and one real bilingual call end to end.
The precise list is in the Plan B doc.
