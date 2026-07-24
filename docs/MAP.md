# MAP — Notetaker (personal)

Fully local macOS notetaker for Mr. Brothers: high-quality dual-track
recording now, idle-time local transcription (EN/ZH, Speaker 1/2/3) and
summarization later, organized by tasks. No cloud.

## State
- **Plan A (portable core): COMPLETE** on branch `plan-a-core` (2026-07-23).
  All 15 tasks done; 75 Rust tests + 6 frontend tests green, including a real
  end-to-end golden test (whisper + sherpa on the bilingual fixture).
- **Plan B (Mac capture layer): not started.** Written when the Mac arrives
  (~2026-07-30); scope registered at the bottom of Plan A. This is what makes
  the app actually record; Plan A is everything downstream of a recording.
- **Spec:** `docs/superpowers/specs/2026-07-23-notetaker-design.md`.
- **Plan A:** `docs/superpowers/plans/2026-07-23-notetaker-v1-core.md`
  (read its "Plan amendment — 2026-07-23" section — WSL2 constraints reshaped
  several tasks).
- **Bake-off decision:** `docs/superpowers/specs/bakeoff-result.md` —
  SenseVoice beats Whisper-tiny on Chinese; it is the default speech engine.

## Layout (as built)
- `src-tauri/core/` — crate `notetaker-core`: ALL portable logic. Modules:
  `storage` (files-first layout), `index` (SQLite FTS5, CJK-segmented search),
  `queue` (crash-safe, 3× retry), `pipeline/` (audio, transcribe, diarize,
  merge, llm, summarize, suggest, run), `models` (registry + downloader),
  `scheduler`, `api` (command logic). Tested via `cargo test -p notetaker-core`
  from `src-tauri/`.
- `src-tauri/` (app crate) — thin Tauri shell. Does NOT compile on Linux
  (needs macOS/webkit); compiled and wired on the Mac in Plan B.
- `src/` — React/TS UI: sidebar, recording list, status chips, detail view,
  speaker rename, search. `src/lib/ipc.ts` is the Rust↔UI contract.
- `fixtures/` — `bilingual.wav` (EN/ZH transcription), `diarization-check.wav`
  (real-human speaker separation), reference transcript.

## Build environment (WSL2, hard-won — see progress.md)
- `cargo test -p notetaker-core` from `src-tauri/` is the only Rust check here.
- Needs `PATH=$HOME/.cargo/bin` and `LIBCLANG_PATH=$HOME/.local/lib/libclang`
  (whisper-rs/sherpa-rs bindgen). `models/` is gitignored; fetch via
  `scripts/fetch-*.sh`.

## Ground rules
- User data layout (`~/Notetaker/Tasks/...`) is a public contract; the SQLite
  index must always be rebuildable from the files.
- Speech = SenseVoice (default) / Whisper (fallback); diarization = sherpa-onnx;
  summaries = Ollama+Qwen. Diarization is verified on real human audio only —
  synthetic TTS voices don't separate.

## Next (Plan B, on the Mac)
Capture engine (dual-track FLAC via ScreenCaptureKit), meeting watcher +
"Record this?", menu bar, real idle/power detection, Metal model builds +
tier auto-detect, Ollama install flow, compile the app crate, DMG packaging,
and re-run the bake-off vs large-v3-turbo.
