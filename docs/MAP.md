# MAP — Notetaker (personal)

Fully local macOS notetaker for Mr. Brothers: high-quality dual-track
recording now, idle-time local transcription (EN/ZH, Speaker 1/2/3) and
summarization later, organized by tasks. No cloud.

## State
- **Phase:** planning complete, implementation not started (2026-07-23).
- **Spec:** `docs/superpowers/specs/2026-07-23-notetaker-design.md` — the
  approved design. Read it first.
- **Plan A (portable core, buildable now on WSL2):**
  `docs/superpowers/plans/2026-07-23-notetaker-v1-core.md`.
- **Plan B (Mac capture layer):** written when the Mac arrives
  (~2026-07-30); scope registered at the bottom of Plan A.

## Intended layout (once built)
- `src-tauri/` — Rust core: capture engine, meeting watcher, processing
  queue, storage. Possible `helper/` Swift sidecar for ScreenCaptureKit.
- `src/` — React/TS UI: menu bar, library, settings.
- `fixtures/` — bilingual multi-speaker test audio for the golden
  pipeline test.

## Ground rules
- Target is macOS; this dev box is WSL2 — code must build cross-platform
  but capture/e2e verification happens on the Mac (arrives ~2026-07-30).
- User data layout (`~/Notetaker/Tasks/...`) is a public contract; the
  SQLite index must always be rebuildable from the files.
- Models: whisper.cpp (Metal), sherpa-onnx diarization, Ollama+Qwen for
  summaries. EN/ZH engine decided by fixture bake-off, not assumption.
