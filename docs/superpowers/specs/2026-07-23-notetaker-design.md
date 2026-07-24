# Notetaker — Design Spec

**Date:** 2026-07-23
**Owner:** Mr. Brothers (product), Claude (build)
**Status:** Approved by Mr. Brothers 2026-07-23 (with in-person recording added)

## 1. What this is

A fully local Mac desktop notetaker. It records meetings and in-person
sessions (classes) in high quality, then — later, when the Mac is idle —
transcribes them with speaker labels (Speaker 1/2/3), handles English and
Chinese mixed in the same recording, and writes a summary using a local AI
model. Recordings are organized into user-defined tasks. Nothing ever leaves
the machine.

Built by transplanting proven parts from open-source projects rather than
forking any of them whole:

- **Meetily** (Tauri + Rust): macOS audio-capture approach (mic + system
  audio), whisper.cpp integration patterns.
- **OpenWhispr** (Electron): local Whisper model management / download UX
  patterns.
- **OBS**: the quality philosophy — lossless, separate tracks, never mix at
  record time. (Approach, not literal code; capture uses Apple's native
  ScreenCaptureKit, same API modern Mac recorders use.)

## 2. Locked product decisions

| Decision | Choice |
|---|---|
| Platform v1 | macOS only (Mr. Brothers moves to Mac ~2026-07-30). Windows is phase 2+. |
| Hardware | Auto-detect on first run; pick model tier accordingly. |
| Summarization AI | App installs Ollama + a default quantized Qwen model itself (best small local models for EN/ZH). Settings allow any OpenAI-compatible local endpoint later. |
| Speakers | Per-recording Speaker 1/2/3, click-to-rename. Cross-meeting voice profiles are phase 2. |
| Auto-record | Detect when a meeting app grabs the mic → "Record this?" prompt (ask-first, not silent auto-start). |
| Task sorting | AI suggests a task after processing; one click to accept or change. |
| Processing schedule | Queue runs when Mac is idle/plugged in; per-recording "Process now" button. |
| In-person mode | Mic-only recording for classes/in-person meetings. All voices (including Mr. Brothers) diarized; he renames himself once per recording. |

## 3. Architecture

Tauri v2 app: Rust core + React/TypeScript UI, plus a small Swift helper
binary for capture if the Rust ScreenCaptureKit bindings prove weaker than
Meetily's in practice (decided in Phase 1 by testing, not assumption).

```
┌───────────────────────────── Menu bar app ─────────────────────────────┐
│  Record meeting ▸   Record room ▸   Open library ▸   (status/queue)    │
└────────────────────────────────────────────────────────────────────────┘
        │                                        │
┌───────▼────────┐   ┌────────────────┐   ┌──────▼──────────────────────┐
│ Capture engine │   │ Meeting watcher │   │ Library UI (main window)   │
│ (Rust/Swift)   │   │ (mic-use poll)  │   │ tasks / recordings / search│
└───────┬────────┘   └────────────────┘   └──────┬──────────────────────┘
        │  FLAC tracks                            │ reads
┌───────▼───────────────────────────────┐  ┌──────▼──────────┐
│ Processing pipeline (idle-time queue) │  │ SQLite index    │
│ diarize → transcribe → merge →        │──│ (metadata + FTS │
│ summarize → suggest task              │  │  search only)   │
└───────┬───────────────────────────────┘  └─────────────────┘
        │ talks to
┌───────▼──────────────────────────────┐
│ Local AI runtime                     │
│ whisper.cpp (Metal) · sherpa-onnx    │
│ diarization · Ollama (Qwen)          │
└──────────────────────────────────────┘
```

## 4. Components

### 4.1 Capture engine
- Two modes:
  - **Meeting:** mic → track A, system audio (ScreenCaptureKit) → track B.
    Synced clocks, started/stopped together.
  - **In person:** mic → single track. Uses best available input; voice
    isolation OFF (we want the whole room).
- Format: FLAC, 48 kHz (lossless; ~15–25 MB per hour per mono track).
  Never mixed, never compressed lossily at record time.
- Crash-safe: audio is flushed to disk continuously; on relaunch after a
  crash, the partial file is finalized and kept.
- Disk-space guard: refuses to start (with a clear message) under a
  configurable free-space floor.
- Pause/resume. Menu-bar timer + colored recording indicator.

### 4.2 Meeting watcher
- Polls CoreAudio for "mic in use" by an allowlist of meeting apps (Zoom,
  Teams, browsers for Meet). On trigger → native notification "Record
  this?" with Start/Ignore. No silent recording.
- Allowlist editable in settings.

### 4.3 Processing pipeline (the "later" part)
Runs from a persistent queue when the Mac is idle or plugged in
(configurable), lowest QoS so fans stay quiet; "Process now" overrides.

1. **Diarization** (sherpa-onnx: pyannote segmentation + speaker
   embeddings, fully local): on the system track (meeting mode) or the mic
   track (in-person mode) → Speaker 1/2/3 segments.
2. **Transcription** (per segment): Whisper large-v3-turbo via whisper.cpp
   with Metal on capable machines; smaller quantized tier otherwise (see
   4.6). Language auto-detect per segment handles EN/ZH switching.
   Build-time bake-off on a real bilingual fixture decides Whisper vs
   SenseVoice (sherpa-onnx) as the EN/ZH engine — whichever transcribes
   mixed-language audio better wins; this spec does not pre-commit.
3. **Merge:** meeting mode — mic track transcribed as "George" (no
   guessing, it is physically his track) and interleaved with the others
   by timestamp. In-person mode — all speakers are diarized labels.
4. **Summarize** (Ollama, Qwen quantized, size picked by RAM): structured
   markdown — TL;DR, key points, decisions, action items, open questions.
   Summary in English by default; keeps load-bearing Chinese quotes.
5. **Task suggestion:** LLM matches the summary against the task list
   (names + descriptions). Low confidence → "Unsorted". One click to
   accept/change in the UI.

Failures retry (3×, backoff) and then surface as a red status chip with a
plain-language error — never silently stuck.

### 4.4 Library UI
- Sidebar: tasks (create/rename/archive). Special views: Unsorted, All,
  Recently processed.
- Recording rows: title, date, duration, mode icon, status chip
  (**Recorded → Queued → Processing → Ready / Failed**), suggested-task
  banner awaiting one-click confirm.
- Detail view: summary (editable), transcript with speaker labels and
  timestamps — click a line to hear that moment; click a speaker name to
  rename everywhere in that recording.
- Search: full-text across transcripts and summaries (SQLite FTS5),
  filterable by task.
- Settings: models/tier, AI endpoint, storage location, auto-record
  allowlist, processing schedule, hotkey.

### 4.5 Storage — plain files first
```
~/Notetaker/
  Tasks/
    Accounting 302/
      2026-08-04 10.02 Lecture 3/
        audio-mic.flac
        audio-system.flac      (meeting mode only)
        transcript.md
        summary.md
        meta.json              (mode, duration, speakers, models used)
  Unsorted/
```
Everything readable in Finder; the app can be deleted and the notes
survive. SQLite (app support dir) is an index only — rebuildable by
rescanning the folder.

### 4.6 Local AI runtime & hardware tiers
- First run: detect chip + RAM →
  - **Apple Silicon ≥ 16 GB:** Whisper large-v3-turbo + Qwen ~8B q4.
  - **Apple Silicon 8 GB:** Whisper small/medium quantized + Qwen ~4B q4.
  - **Intel Mac:** CPU quantized small tier; warn about processing speed.
- Ollama: detect existing install; else install + pull default model with
  progress UI. Model downloads resumable, checksum-verified.
- Whisper/diarization models downloaded on first run with the same UX
  (OpenWhispr's pattern).

## 5. Error handling summary
- Recording: crash-recovery finalize; disk guard; input-device loss →
  pause + notify (never silently record nothing).
- Processing: retries then visible failure state; per-stage logging kept
  next to the recording in `meta.json`.
- Models: download verification; missing model → clear re-download action,
  never a crash.

## 6. Testing & verification
- Rust unit tests (capture state machine, queue, storage naming).
- Golden pipeline test: a checked-in bilingual EN/ZH multi-speaker fixture
  clip must produce ≥2 speakers and both languages in the transcript.
- UI: component tests + screenshot verification against stated acceptance
  beats for each slice.
- Every phase ends with a machine-verified acceptance check (Mr. Brothers
  does not review code).

## 7. Phases
- **V1 (this plan):** everything above.
- **Phase 2 candidates (explicitly out of v1):** voice profiles across
  meetings, Windows build, calendar awareness, live transcription, mobile
  companion, translation of summaries to Chinese.

## 8. To verify during build (assumptions, not facts)
- Meetily's current Rust capture code quality vs a thin Swift SCK sidecar.
- sherpa-onnx diarization model availability/licensing for bundling.
- Whisper large-v3-turbo vs SenseVoice on real code-switched EN/ZH audio.
- Ollama silent-install UX on macOS.
