# Notetaker V1 — Plan A: Portable Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build every platform-portable part of the Notetaker — storage, search index, processing queue, the full diarize→transcribe→merge→summarize→suggest pipeline, and the library UI — verifiable on Linux/WSL2 now, so Plan B (Mac capture layer) only adds recording on top.

**Architecture:** Tauri v2 app; Rust core owns storage/queue/pipeline, React+TS owns UI, SQLite FTS5 is a rebuildable index over plain files in `~/Notetaker/`. Pipeline stages are traits with local-model implementations (whisper.cpp via whisper-rs, sherpa-onnx via sherpa-rs, Ollama via HTTP).

**Tech Stack:** Rust (stable), Tauri 2, React 18 + TypeScript + Vite, rusqlite (bundled SQLite + FTS5), whisper-rs, sherpa-rs, ureq, serde, hound/symphonia, vitest + @testing-library/react, espeak-ng + sox (fixture generation only).

## Global Constraints

- All Plan-A code MUST build and pass tests on Linux (WSL2) AND compile for macOS. No `#[cfg(target_os)]` without a Linux-testable fallback.
- No network calls at runtime except: localhost Ollama, and model downloads from URLs in `src-tauri/src/models/registry.rs`. Nothing else, ever (spec: fully local).
- User data layout is a public contract (spec §4.5): `~/Notetaker/Tasks/<task>/<YYYY-MM-DD HH.MM <title>>/{audio-mic.flac, audio-system.flac, transcript.md, summary.md, meta.json}` plus `~/Notetaker/Unsorted/`. SQLite index MUST be rebuildable by rescanning that tree.
- Recording statuses, exactly: `recorded → queued → processing → ready | failed` (spec §4.4).
- Meeting mode: mic track is always labeled `George`, never diarized. In-person mode: single track, all speakers diarized (spec §4.3).
- Add crate deps with `cargo add <crate>` (resolver picks versions — do NOT hand-pin versions from this plan). JS deps with `pnpm add`.
- Crate/API details in code blocks were written from documentation memory; if a signature drifted, fix at the call site and keep the task's OWN interfaces exactly as written — later tasks depend on them.
- Conventional commits (`feat:`, `test:`, `chore:`). Commit at the end of every task minimum.
- Storage root is configurable; every module takes it as a parameter. Tests use `tempfile::tempdir()`, never the real `~/Notetaker`.

### Plan amendment — 2026-07-23 (WSL2 has no sudo; webkit/gtk dev libs uninstallable)

Verified twice (implementer + controller `sudo -n`): system packages cannot be installed this week. Amendments, which every task brief inherits:

1. **Workspace split.** `src-tauri/Cargo.toml` becomes a workspace (`members = [".", "core"]`). A new library crate **`notetaker-core`** lives at `src-tauri/core/` and carries ALL Rust logic. Wherever a task says `src-tauri/src/<module>`, write **`src-tauri/core/src/<module>`**; `crate::X` paths in task code remain valid inside core. The app crate (`src-tauri/src/`) holds only Tauri glue.
2. **Test command.** "cargo test" in any task means **`cargo test -p notetaker-core`** (run from `src-tauri/`). The app crate does not compile on this Linux box (missing webkit system libs); its compile check + `pnpm tauri dev` are registered as Plan B items on the Mac.
3. **Task 5.** espeak-ng and sox are unavailable. Generate the fixture with **piper-tts** (pip, local ONNX voices: one `en_US` medium voice + one `zh_CN` voice) and **ffmpeg** (present) for resample/concat. Output contract unchanged: `fixtures/bilingual.wav`, 16 kHz mono, two clearly different voices, EN + ZH content.
4. **Task 12.** Command handler logic goes in `notetaker-core` as `core/src/api.rs` (fully tested on Linux). The `#[tauri::command]` wrappers in the app crate are thin one-liners delegating to core; their compile check is a Plan B Mac item.
5. **No pkg-config on this box:** core crates must avoid native-lib discovery — rusqlite stays on `bundled`, HTTP stays on `ureq` (rustls). This was already the plan; it is now a hard constraint.
6. **libclang for bindgen (whisper-rs / sherpa-rs).** No system libclang and no sudo. Resolved without root: a user-space copy lives at `~/.local/lib/libclang/libclang.so` (obtained via `uv pip install libclang`). Any task touching whisper-rs or sherpa-rs must export `LIBCLANG_PATH=$HOME/.local/lib/libclang`. Verified: `whisper-rs` 0.16 compiles clean this way in 46 s. **Do not commit this path** to `.cargo/config.toml` — on the Mac, libclang ships with Xcode CLT and a hardcoded Linux path would break the build.
7. **Chinese search requires CJK segmentation (Task 3 spec change).** Measured on this box with bundled SQLite: FTS5's default `unicode61` tokenizer treats a run of Han characters as ONE token, so searching `预算` inside `我们今天讨论预算和招聘计划` returns **0 hits** — Chinese search silently does not work. The `trigram` tokenizer fixes 3+ character terms but still fails 2-character words like `预算` (the most common Chinese word length). **Ship this instead:** keep the default tokenizer and insert spaces around every CJK codepoint (`U+3400–4DBF`, `U+4E00–9FFF`, `U+F900–FAFF`) when indexing AND when building the query, then match as a quoted phrase. Verified: `预算`, `招聘计划`, `budget`, and `quarterly budget` all return hits. Task 3 must implement this as `fn segment_cjk(&str) -> String` applied on both sides, with a test asserting a two-character Chinese term is found inside a longer Chinese sentence.
8. **Parallel execution.** Tasks run concurrently in isolated git worktrees under `/home/georg/projects/notetaker-wt/tN` (branch `tN`), merged back to `plan-a-core` per wave. To make that safe, one prep commit (`d60d805`) pre-added every core dependency, created every module stub, and placed the cross-task types (`pipeline::Utterance`, `pipeline::diarize::SpeakerSpan`) — so no two agents ever edit the same file. Agents must not run `cargo add` or edit `Cargo.toml`/`lib.rs`/`pipeline/mod.rs`.

---

### Task 1: Scaffold Tauri app + test harnesses

**Files:**
- Create: entire app skeleton at repo root (`src/`, `src-tauri/`, `package.json`, `vite.config.ts`)
- Test: `src-tauri/src/lib.rs` (smoke test), `src/App.test.tsx`

**Interfaces:**
- Produces: a repo where `cargo test` (in `src-tauri/`) and `pnpm test` both run green; `src-tauri/src/lib.rs` is the crate root all later Rust modules hang off.

- [ ] **Step 1: Scaffold**

```bash
cd "/home/georg/projects/notetaker personal"
pnpm create tauri-app@latest . --template react-ts --manager pnpm --yes
pnpm install
```
If the CLI refuses a non-empty dir, scaffold in `/tmp/claude-1000/.../scratchpad/nt` and move contents in (keep existing `docs/` and `.git/`).

- [ ] **Step 2: Add Rust smoke test**

In `src-tauri/src/lib.rs` append:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn harness_works() { assert_eq!(2 + 2, 4); }
}
```

- [ ] **Step 3: Add frontend test harness**

```bash
pnpm add -D vitest @testing-library/react @testing-library/jest-dom jsdom
```

`src/App.test.tsx`:
```tsx
import { describe, it, expect } from "vitest";
describe("harness", () => { it("works", () => expect(true).toBe(true)); });
```

Add to `package.json` scripts: `"test": "vitest run"`. Add to `vite.config.ts`: `test: { environment: "jsdom" }` (with `/// <reference types="vitest" />`).

- [ ] **Step 4: Verify**

Run: `cd src-tauri && cargo test` → Expected: `harness_works ... ok`.
Run: `pnpm test` → Expected: 1 passed.
(`pnpm tauri dev` needs a display; do not require it for verification on WSL2.)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: scaffold tauri v2 app with rust+vitest test harnesses"
```

---

### Task 2: Storage module — folders, meta.json, scan

**Files:**
- Create: `src-tauri/src/storage/mod.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod storage;`)

**Interfaces:**
- Produces (later tasks import these exact items from `crate::storage`):

```rust
pub enum Mode { Meeting, InPerson }
pub enum Status { Recorded, Queued, Processing, Ready, Failed }
pub struct Meta {
    pub id: String,            // uuid v4
    pub title: String,
    pub mode: Mode,
    pub created: String,       // RFC3339
    pub duration_s: f64,
    pub status: Status,
    pub speakers: std::collections::BTreeMap<String, String>, // "spk1" -> display name
    pub error: Option<String>,
    pub attempts: u32,
}
pub struct RecordingRef { pub meta: Meta, pub dir: std::path::PathBuf, pub task: Option<String> } // task None = Unsorted
pub struct Store { pub root: std::path::PathBuf }
impl Store {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self;
    pub fn create_recording(&self, title: &str, mode: Mode, created: chrono::DateTime<chrono::Local>) -> anyhow::Result<RecordingRef>; // creates Unsorted/<YYYY-MM-DD HH.MM title>/meta.json, status Recorded
    pub fn save_meta(&self, rec: &RecordingRef) -> anyhow::Result<()>;
    pub fn scan(&self) -> anyhow::Result<Vec<RecordingRef>>;      // walks Tasks/* and Unsorted/
    pub fn list_tasks(&self) -> anyhow::Result<Vec<String>>;      // dir names under Tasks/
    pub fn create_task(&self, name: &str) -> anyhow::Result<()>;
    pub fn assign_task(&self, rec: &RecordingRef, task: &str) -> anyhow::Result<RecordingRef>; // moves the folder
}
```

- [ ] **Step 1: Deps + failing tests**

```bash
cd src-tauri && cargo add serde --features derive && cargo add serde_json anyhow chrono uuid --features uuid/v4 && cargo add tempfile --dev
```

Bottom of `src-tauri/src/storage/mod.rs` (write tests first, types referenced don't exist yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        (d.into(), Store::new(d.path())) // adjust: keep TempDir alive alongside Store
    }
    #[test]
    fn create_lands_in_unsorted_with_meta() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::InPerson, created).unwrap();
        assert!(r.dir.ends_with("Unsorted/2026-08-04 10.02 Lecture 3"));
        assert!(r.dir.join("meta.json").exists());
        assert!(matches!(r.meta.status, Status::Recorded));
        assert!(r.task.is_none());
    }
    #[test]
    fn scan_roundtrips_and_assign_moves() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path());
        s.create_task("Accounting 302").unwrap();
        let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
        let r = s.create_recording("Lecture 3", Mode::Meeting, created).unwrap();
        let r = s.assign_task(&r, "Accounting 302").unwrap();
        assert_eq!(r.task.as_deref(), Some("Accounting 302"));
        let all = s.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta.title, "Lecture 3");
        assert_eq!(s.list_tasks().unwrap(), vec!["Accounting 302"]);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test storage` → compile errors (types undefined). Expected.

- [ ] **Step 3: Implement**

Implementation notes (write real code, this is the shape):
- `Meta` derives `Serialize, Deserialize, Clone, Debug`; `Mode`/`Status` use `#[serde(rename_all = "snake_case")]`.
- Folder name: `format!("{} {}", created.format("%Y-%m-%d %H.%M"), sanitize(title))` where `sanitize` strips `/:\\*?"<>|` and trims. On name collision append ` (2)`, ` (3)`.
- `scan` = for each dir in `Tasks/*/*` and `Unsorted/*` containing `meta.json`, parse it; skip unparseable dirs with a `log::warn!`, never abort the whole scan.
- `assign_task` = `fs::rename` into `Tasks/<task>/`, then re-derive `RecordingRef`.

- [ ] **Step 4: Verify** — `cargo test storage` → Expected: 2 passed.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: storage module with files-first layout, scan, task assignment"`

---

### Task 3: SQLite index (FTS5) — rebuildable from disk

**Files:**
- Create: `src-tauri/src/index/mod.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod index;`)

**Interfaces:**
- Consumes: `crate::storage::{Store, RecordingRef}`.
- Produces:

```rust
pub struct Index { /* rusqlite::Connection */ }
pub struct SearchHit { pub id: String, pub title: String, pub task: Option<String>, pub snippet: String }
impl Index {
    pub fn open(db_path: &std::path::Path) -> anyhow::Result<Index>;
    pub fn rebuild(&mut self, store: &crate::storage::Store) -> anyhow::Result<usize>; // full rescan; reads transcript.md/summary.md if present
    pub fn upsert(&mut self, rec: &crate::storage::RecordingRef, transcript: &str, summary: &str) -> anyhow::Result<()>;
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>>;
}
```

- [ ] **Step 1: Dep + failing test**

```bash
cargo add rusqlite --features bundled
```

Schema (in `Index::open`, `CREATE TABLE IF NOT EXISTS`):
```sql
CREATE TABLE IF NOT EXISTS recordings(
  id TEXT PRIMARY KEY, title TEXT, task TEXT, created TEXT,
  duration_s REAL, mode TEXT, status TEXT, dir TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS recordings_fts USING fts5(id UNINDEXED, title, transcript, summary);
```

Test:
```rust
#[test]
fn rebuild_indexes_transcripts_and_survives_db_delete() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::new(dir.path().join("root"));
    let created = chrono::Local.with_ymd_and_hms(2026, 8, 4, 10, 2, 0).unwrap();
    let r = s.create_recording("Budget sync", Mode::Meeting, created).unwrap();
    std::fs::write(r.dir.join("transcript.md"), "George: the quarterly budget is late").unwrap();
    let db = dir.path().join("ix.sqlite");
    let mut ix = Index::open(&db).unwrap();
    assert_eq!(ix.rebuild(&s).unwrap(), 1);
    assert_eq!(ix.search("quarterly budget").unwrap()[0].title, "Budget sync");
    drop(ix); std::fs::remove_file(&db).unwrap();          // index is disposable
    let mut ix2 = Index::open(&db).unwrap();
    ix2.rebuild(&s).unwrap();
    assert_eq!(ix2.search("quarterly").unwrap().len(), 1); // proves rebuildability
}
```

- [ ] **Step 2: Run** — `cargo test index` → compile fail. Expected.
- [ ] **Step 3: Implement** — `search` uses `snippet(recordings_fts, 2, '<b>', '</b>', '…', 12)` joined to `recordings` on id; query goes through FTS5 `MATCH` with user input escaped by wrapping in double quotes per token.
- [ ] **Step 4: Verify** — `cargo test index` → 1 passed.
- [ ] **Step 5: Commit** — `git commit -am "feat: sqlite fts5 index, rebuildable from files"`

---

### Task 4: Processing queue — states, persistence, retry

**Files:**
- Create: `src-tauri/src/queue/mod.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod queue;`)

**Interfaces:**
- Consumes: `crate::storage::{Store, RecordingRef, Status, Meta}`.
- Produces:

```rust
pub trait IdleSource: Send + Sync { fn ok_to_run(&self) -> bool; } // Plan B adds real mac impl
pub struct AlwaysIdle;                                            // Linux/test impl
pub struct Queue<'a> { pub store: &'a crate::storage::Store }
impl<'a> Queue<'a> {
    pub fn enqueue(&self, rec: &mut crate::storage::RecordingRef) -> anyhow::Result<()>;   // recorded -> queued
    pub fn next(&self) -> anyhow::Result<Option<crate::storage::RecordingRef>>;            // oldest queued
    pub fn run_one<F>(&self, idle: &dyn IdleSource, process: F) -> anyhow::Result<RunOutcome>
        where F: FnOnce(&crate::storage::RecordingRef) -> anyhow::Result<()>;
}
pub enum RunOutcome { Ran, NothingQueued, NotIdle, FailedWillRetry, FailedFinal }
```

State rules (test each): `enqueue` only from `Recorded`/`Failed`; `run_one` sets `Processing`, on Ok sets `Ready`, on Err increments `attempts` — attempts < 3 → back to `Queued` (`FailedWillRetry`), attempts ≥ 3 → `Failed` with `error` set (`FailedFinal`). Queue order/persistence come from meta.json on disk (crash-safe, no extra state file).

- [ ] **Step 1: Failing tests** — write tests for: happy path recorded→queued→processing→ready; failure retries twice then `FailedFinal` with `meta.error` containing the error string; `NotIdle` when `idle.ok_to_run() == false` leaves status `Queued`; `next()` returns oldest first (create two, assert order by `created`).
- [ ] **Step 2: Run** — `cargo test queue` → compile fail. Expected.
- [ ] **Step 3: Implement** — status transitions written to disk via `store.save_meta` BEFORE and AFTER the closure runs (so a crash mid-process leaves `Processing`, and a startup sweep in Task 11 requeues stale `Processing` rows).
- [ ] **Step 4: Verify** — `cargo test queue` → 4 passed.
- [ ] **Step 5: Commit** — `git commit -am "feat: crash-safe processing queue with 3x retry"`

---

### Task 5: Bilingual test fixture (generated, committed)

**Files:**
- Create: `fixtures/make_fixture.sh`, `fixtures/bilingual.wav` (committed output), `fixtures/README.md`

**Interfaces:**
- Produces: `fixtures/bilingual.wav` — 16 kHz mono WAV, ~40 s, two clearly different synthetic voices, English AND Mandarin content. Used by Tasks 6, 7, 11.

- [ ] **Step 1: Install generators** — `sudo apt-get install -y espeak-ng sox` (if sudo unavailable, note it and generate on any machine, only the wav is consumed).

- [ ] **Step 2: Write `fixtures/make_fixture.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
T=$(mktemp -d)
espeak-ng -v en-us+m3 -s 150 -w "$T/a1.wav" "Good morning everyone. Today we will review the quarterly budget and the hiring plan."
espeak-ng -v cmn+f4  -s 150 -w "$T/b1.wav" "大家好。我们今天讨论预算和招聘计划。这个季度的收入增长了百分之十。"
espeak-ng -v en-us+m3 -s 150 -w "$T/a2.wav" "That is great news. Let us schedule the follow up meeting for next Tuesday."
espeak-ng -v cmn+f4  -s 150 -w "$T/b2.wav" "好的，没问题。下周二上午十点可以吗。"
sox "$T/a1.wav" -r 16000 -c 1 "$T/A1.wav"; sox "$T/b1.wav" -r 16000 -c 1 "$T/B1.wav"
sox "$T/a2.wav" -r 16000 -c 1 "$T/A2.wav"; sox "$T/b2.wav" -r 16000 -c 1 "$T/B2.wav"
sox -n -r 16000 -c 1 "$T/sil.wav" trim 0.0 0.8
sox "$T/A1.wav" "$T/sil.wav" "$T/B1.wav" "$T/sil.wav" "$T/A2.wav" "$T/sil.wav" "$T/B2.wav" bilingual.wav
echo "wrote $(pwd)/bilingual.wav"
```

- [ ] **Step 3: Generate + verify** — `chmod +x fixtures/make_fixture.sh && ./fixtures/make_fixture.sh` then `soxi fixtures/bilingual.wav` → Expected: 16000 Hz, 1 channel, duration 25–60 s.
- [ ] **Step 4: Contingency note in `fixtures/README.md`** — if the diarizer in Task 7 cannot separate these two synthetic voices, replace the file with a short CC-licensed real two-speaker EN/ZH clip and record its provenance here; the test interface (path + "≥2 speakers, both languages") does not change.
- [ ] **Step 5: Commit** — `git add fixtures && git commit -m "test: generated bilingual two-voice fixture"`

---

### Task 6: Audio decode + transcription stage (whisper.cpp)

**Files:**
- Create: `src-tauri/src/pipeline/mod.rs`, `src-tauri/src/pipeline/audio.rs`, `src-tauri/src/pipeline/transcribe.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod pipeline;`)

**Interfaces:**
- Produces (all later pipeline tasks use these):

```rust
// pipeline/mod.rs
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Utterance { pub start_s: f32, pub end_s: f32, pub speaker: String, pub text: String }
// pipeline/audio.rs
pub fn load_mono_16k(path: &std::path::Path) -> anyhow::Result<Vec<f32>>; // wav now, flac via symphonia
// pipeline/transcribe.rs
pub trait Transcriber { fn transcribe(&self, samples: &[f32], spans: &[(f32, f32)]) -> anyhow::Result<Vec<(f32, f32, String)>>; }
pub struct WhisperTranscriber { /* whisper-rs context */ }
impl WhisperTranscriber { pub fn load(model_path: &std::path::Path) -> anyhow::Result<Self>; }
```
`spans` = time ranges to transcribe (from diarization); empty slice means "whole file as one span". Language: auto-detect (`set_language(None)`) so EN/ZH both come out.

- [ ] **Step 1: Deps** — `cargo add whisper-rs hound symphonia` (symphonia with `flac` feature).
- [ ] **Step 2: Failing test** (ignored-by-default gate on model file):

```rust
#[test]
fn transcribes_fixture_in_two_languages() {
    let model = std::path::Path::new("../models/ggml-tiny.bin"); // multilingual tiny, ~75MB
    if !model.exists() { eprintln!("SKIP: run scripts/fetch-test-models.sh"); return; }
    let samples = crate::pipeline::audio::load_mono_16k(std::path::Path::new("../fixtures/bilingual.wav")).unwrap();
    let t = WhisperTranscriber::load(model).unwrap();
    let out = t.transcribe(&samples, &[]).unwrap();
    let all: String = out.iter().map(|(_, _, s)| s.as_str()).collect::<Vec<_>>().join(" ");
    assert!(all.to_lowercase().contains("budget"), "english missing: {all}");
    assert!(all.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "chinese missing: {all}");
}
```

Create `scripts/fetch-test-models.sh` downloading `ggml-tiny.bin` from the official whisper.cpp Hugging Face mirror into `models/` (gitignored) with `curl -L -C -` and a sha256 check.

- [ ] **Step 3: Run** — `cargo test transcribe` → compile fail, then (after stubs) SKIP or FAIL. Expected.
- [ ] **Step 4: Implement** — `load_mono_16k`: hound for WAV (i16 → f32 /32768, average channels, error if not 16 kHz for now — resample lands with FLAC support); whisper-rs: `WhisperContext::new_with_params`, `FullParams::new(SamplingStrategy::Greedy { best_of: 1 })`, `set_language(None)`, `set_translate(false)`; for each span slice `samples[(start*16000.)as usize..]`, run `state.full`, collect segments offset by span start. Whole-file when spans empty. If tiny-model quality makes the "budget" assertion flaky, assert on Chinese chars + `out.len() >= 2` and note it — the golden accuracy gate is Task 11's with a bigger model.
- [ ] **Step 5: Verify** — `bash scripts/fetch-test-models.sh && cargo test transcribe` → 1 passed.
- [ ] **Step 6: Commit** — `git commit -am "feat: whisper.cpp transcription stage with EN/ZH auto-detect"`

---

### Task 7: Diarization stage (sherpa-onnx)

**Files:**
- Create: `src-tauri/src/pipeline/diarize.rs`
- Modify: `scripts/fetch-test-models.sh` (add diarization models)

**Interfaces:**
- Produces:

```rust
pub struct SpeakerSpan { pub start_s: f32, pub end_s: f32, pub speaker: u32 } // 0-based
pub trait Diarizer { fn diarize(&self, samples: &[f32]) -> anyhow::Result<Vec<SpeakerSpan>>; }
pub struct SherpaDiarizer { /* sherpa-rs offline speaker diarization */ }
impl SherpaDiarizer { pub fn load(segmentation_onnx: &std::path::Path, embedding_onnx: &std::path::Path) -> anyhow::Result<Self>; }
```

- [ ] **Step 1: Dep** — `cargo add sherpa-rs`. Models for the fetch script (both hosted in k2-fsa's sherpa-onnx releases; confirm exact URLs from the sherpa-onnx "speaker-diarization" docs page at implementation time): `sherpa-onnx-pyannote-segmentation-3-0.onnx` + a 3D-Speaker embedding model (e.g. `3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` — Chinese-trained embeddings, fine for language-agnostic voiceprints).
- [ ] **Step 2: Failing test** — same skip-if-no-model pattern as Task 6:

```rust
#[test]
fn fixture_yields_at_least_two_speakers() {
    // skip guard on model files, then:
    let samples = crate::pipeline::audio::load_mono_16k(std::path::Path::new("../fixtures/bilingual.wav")).unwrap();
    let d = SherpaDiarizer::load(seg_path, emb_path).unwrap();
    let spans = d.diarize(&samples).unwrap();
    let speakers: std::collections::BTreeSet<u32> = spans.iter().map(|s| s.speaker).collect();
    assert!(speakers.len() >= 2, "expected >=2 speakers, got {:?}", speakers);
    assert!(spans.iter().all(|s| s.end_s > s.start_s));
}
```
- [ ] **Step 3: Run** — compile fail / FAIL. Expected.
- [ ] **Step 4: Implement** — wrap sherpa-rs's offline speaker diarization API (consult sherpa-rs `examples/` — the crate ships a diarization example; keep OUR trait exactly as above so a binding change never leaks past this file). If synthetic voices won't separate: apply Task 5's contingency (real clip), do not weaken the ≥2 assertion.
- [ ] **Step 5: Verify** — `cargo test diarize` → 1 passed.
- [ ] **Step 6: Commit** — `git commit -am "feat: local speaker diarization via sherpa-onnx"`

---

### Task 8: Merge stage (pure logic)

**Files:**
- Create: `src-tauri/src/pipeline/merge.rs`

**Interfaces:**
- Consumes: `Utterance` (Task 6), `SpeakerSpan` (Task 7).
- Produces:

```rust
pub fn label_speakers(spans: &[SpeakerSpan], texts: &[(f32, f32, String)]) -> Vec<Utterance>; // speaker N -> "Speaker N+1"
pub fn merge_meeting(mic: Vec<Utterance>, others: Vec<Utterance>) -> Vec<Utterance>;          // mic pre-labeled "George"; stable sort by start_s
pub fn to_transcript_md(title: &str, utts: &[Utterance]) -> String;                            // "[HH:MM:SS] **Name:** text" lines
```

- [ ] **Step 1: Failing tests** — pure functions, no models, no skips:

```rust
#[test]
fn meeting_merge_interleaves_by_time() {
    let mic = vec![u("George", 5.0, 8.0, "I agree")];
    let others = vec![u("Speaker 1", 0.0, 4.0, "大家好"), u("Speaker 2", 9.0, 12.0, "Next item")];
    let m = merge_meeting(mic, others);
    assert_eq!(m.iter().map(|x| x.speaker.as_str()).collect::<Vec<_>>(), ["Speaker 1", "George", "Speaker 2"]);
}
#[test]
fn transcript_md_formats_timestamps() {
    let md = to_transcript_md("T", &[u("George", 3661.5, 3665.0, "hi")]);
    assert!(md.contains("[01:01:01] **George:** hi"), "{md}");
}
#[test]
fn label_speakers_assigns_span_majority_overlap() {
    let spans = vec![sp(0, 0.0, 5.0), sp(1, 5.0, 10.0)];
    let texts = vec![(0.5, 4.5, "hello".into()), (5.5, 9.0, "你好".into())];
    let out = label_speakers(&spans, &texts);
    assert_eq!(out[0].speaker, "Speaker 1");
    assert_eq!(out[1].speaker, "Speaker 2");
}
```
(`u`/`sp` are 3-line test helper constructors — write them in the test module.)

- [ ] **Step 2: Run** — compile fail. Expected.
- [ ] **Step 3: Implement** — `label_speakers`: for each text span pick the speaker whose diarization span overlaps it most (interval intersection); no overlap → nearest span. `merge_meeting`: concat + stable sort by `start_s`. `to_transcript_md`: `format!("[{:02}:{:02}:{:02}] **{}:** {}", ...)`.
- [ ] **Step 4: Verify** — `cargo test merge` → 3 passed.
- [ ] **Step 5: Commit** — `git commit -am "feat: speaker labeling and meeting/in-person transcript merge"`

---

### Task 9: Summarize + task-suggest stages (Ollama client)

**Files:**
- Create: `src-tauri/src/pipeline/llm.rs` (client), `src-tauri/src/pipeline/summarize.rs`, `src-tauri/src/pipeline/suggest.rs`

**Interfaces:**
- Produces:

```rust
// llm.rs
pub struct LlmClient { pub base_url: String, pub model: String }  // base_url default "http://localhost:11434"
impl LlmClient { pub fn chat(&self, system: &str, user: &str) -> anyhow::Result<String>; } // POST {base}/api/chat, stream:false
// summarize.rs
pub fn summarize(llm: &LlmClient, transcript_md: &str) -> anyhow::Result<String>; // markdown: ## TL;DR / Key points / Decisions / Action items / Open questions
// suggest.rs
pub struct Suggestion { pub task: Option<String>, pub confidence: f32 } // None => Unsorted
pub fn suggest_task(llm: &LlmClient, summary: &str, tasks: &[String]) -> anyhow::Result<Suggestion>;
```

- [ ] **Step 1: Deps** — `cargo add ureq --features json` and `cargo add httpmock --dev`.
- [ ] **Step 2: Failing tests** — httpmock server plays Ollama:

```rust
#[test]
fn chat_posts_ollama_shape_and_returns_content() {
    let server = httpmock::MockServer::start();
    let m = server.mock(|when, then| {
        when.method(POST).path("/api/chat").json_body_partial(r#"{"stream": false}"#);
        then.status(200).json_body(serde_json::json!({"message": {"role": "assistant", "content": "## TL;DR\nhi"}}));
    });
    let llm = LlmClient { base_url: server.base_url(), model: "qwen3:8b".into() };
    assert!(llm.chat("sys", "user").unwrap().contains("TL;DR"));
    m.assert();
}
#[test]
fn suggest_parses_json_and_applies_threshold() {
    // mock returns {"task": "Accounting 302", "confidence": 0.41}
    // assert Suggestion.task == None because 0.41 < 0.6 threshold
    // second mock returns 0.9 -> Some("Accounting 302")
    // third: model returns a task NOT in the provided list -> None (never invent tasks)
}
#[test]
fn ollama_down_is_a_clean_error() {
    let llm = LlmClient { base_url: "http://127.0.0.1:1".into(), model: "x".into() };
    let e = llm.chat("s", "u").unwrap_err().to_string();
    assert!(e.to_lowercase().contains("ollama"), "error must name ollama for the UI: {e}");
}
```
- [ ] **Step 3: Run** — compile fail. Expected.
- [ ] **Step 4: Implement** — summarize system prompt (exact copy):
  > You are a meticulous meeting-notes assistant. Summarize the transcript into markdown with sections: ## TL;DR (2-3 sentences), ## Key points, ## Decisions, ## Action items (checkbox list with owner names from the transcript), ## Open questions. Write in English; keep short Chinese quotes verbatim where the original wording matters. Do not invent facts not in the transcript.

  suggest system prompt (exact copy):
  > Pick which task this recording belongs to. Reply with ONLY a JSON object {"task": string, "confidence": number 0-1}. The task MUST be one of the provided list, or "" if none fit.

  `suggest_task` parses JSON leniently (strip code fences), rejects tasks not in the list, threshold `>= 0.6`.
- [ ] **Step 5: Verify** — `cargo test llm summarize suggest` → all passed.
- [ ] **Step 6: Commit** — `git commit -am "feat: ollama summarization and task suggestion with confidence gate"`

---

### Task 10: Model manager — registry, resumable downloads, tiers

**Files:**
- Create: `src-tauri/src/models/mod.rs`, `src-tauri/src/models/registry.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod models;`)

**Interfaces:**
- Produces:

```rust
pub struct ModelSpec { pub name: &'static str, pub url: &'static str, pub sha256: &'static str, pub dest: &'static str }
pub enum Tier { AppleSiliconBig, AppleSiliconSmall, CpuSmall }
pub fn detect_tier(total_ram_gb: u64, is_apple_silicon: bool) -> Tier; // >=16GB AS -> Big; AS -> Small; else CpuSmall
pub fn required_models(tier: &Tier) -> Vec<&'static ModelSpec>;
pub struct Downloader { pub models_dir: std::path::PathBuf }
impl Downloader {
    pub fn ensure<F: FnMut(u64, u64)>(&self, spec: &ModelSpec, progress: F) -> anyhow::Result<std::path::PathBuf>;
    // resumes with HTTP Range if partial file exists; verifies sha256; deletes+errors on mismatch
}
```
Registry entries: whisper large-v3-turbo ggml (Big), whisper small-q5 ggml (Small/Cpu), the two diarization onnx files (all tiers). Fill real URLs + hashes from the whisper.cpp HF repo / sherpa-onnx releases at implementation time and hardcode them — the registry file IS the allowlist the Global Constraints refer to.

- [ ] **Step 1: Deps** — `cargo add sha2 sysinfo`.
- [ ] **Step 2: Failing tests** — httpmock serving a 1 KB "model": full download writes file + passes checksum; corrupted body → error mentioning "checksum" and partial file removed; pre-existing half file → request carries `Range: bytes=N-` (assert via httpmock); `detect_tier(16, true) == AppleSiliconBig`, `(8, true) == AppleSiliconSmall`, `(32, false) == CpuSmall`.
- [ ] **Step 3: Run** — compile fail. Expected.
- [ ] **Step 4: Implement** — stream to `<dest>.part`, rename on verified completion (atomic-ish; a crash leaves a resumable .part).
- [ ] **Step 5: Verify** — `cargo test models` → all passed.
- [ ] **Step 6: Commit** — `git commit -am "feat: model registry with resumable checksummed downloads and hw tiers"`

---

### Task 11: Pipeline orchestrator + golden test

**Files:**
- Create: `src-tauri/src/pipeline/run.rs`
- Modify: `src-tauri/src/pipeline/mod.rs` (re-exports)

**Interfaces:**
- Consumes: everything from Tasks 2,4,6,7,8,9.
- Produces:

```rust
pub struct PipelineDeps<'a> {
    pub transcriber: &'a dyn crate::pipeline::transcribe::Transcriber,
    pub diarizer: &'a dyn crate::pipeline::diarize::Diarizer,
    pub llm: &'a crate::pipeline::llm::LlmClient,
    pub tasks: Vec<String>,
}
pub fn process_recording(deps: &PipelineDeps, rec: &crate::storage::RecordingRef) -> anyhow::Result<ProcessOutput>;
pub struct ProcessOutput { pub transcript_md: String, pub summary_md: String, pub suggestion: crate::pipeline::suggest::Suggestion }
// Writes transcript.md + summary.md into rec.dir, fills meta.speakers ("spk1" -> "Speaker 1", plus "george" -> "George" in meeting mode),
// stage timings appended to meta.json under "stages". Caller (queue closure) owns status changes.
```
Mode logic: meeting → diarize+transcribe `audio-system.flac`, transcribe `audio-mic.flac` whole as George, `merge_meeting`; in-person → diarize+transcribe `audio-mic.flac` only. Missing system file in meeting mode = hard error (never silently degrade).

- [ ] **Step 1: Failing golden test** — build a fake recording dir with `fixtures/bilingual.wav` copied in as `audio-mic.flac`'s stand-in (name it `audio-mic.wav`; `load_mono_16k` dispatches on extension), mode in-person, real tiny-whisper + real diarizer (skip-guarded), httpmock LLM. Assert: `transcript.md` written with ≥2 distinct `**Speaker N:**` labels AND Chinese characters AND ASCII words; `summary.md` written; suggestion honored.
- [ ] **Step 2: Run** — FAIL. Expected.
- [ ] **Step 3: Implement** — straight-line orchestration; wrap each stage with `meta.stages` timing entries; every stage error is `anyhow::Context`-tagged with the stage name (queue surfaces it to the UI).
- [ ] **Step 4: Verify** — `cargo test golden` → 1 passed. **This is the spec §6 golden gate.**
- [ ] **Step 5: Also add startup sweep** — `pub fn requeue_stale(store) -> Result<usize>`: any `Processing` meta at app start → `Queued` (crash recovery). Unit test with a hand-written meta.
- [ ] **Step 6: Commit** — `git commit -am "feat: pipeline orchestrator with golden bilingual test and crash sweep"`

---

### Task 12: Tauri IPC commands + app state

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (register commands, managed state: `Store`, `Index`, settings)
- Create: `src/lib/ipc.ts`

**Interfaces:**
- Produces Tauri commands (exact names, snake_case) and a typed TS mirror in `ipc.ts`:

```
list_tasks() -> string[]                     create_task(name: string) -> void
list_recordings() -> RecordingRow[]          get_recording(id: string) -> RecordingDetail
search(query: string) -> SearchHit[]         process_now(id: string) -> void
assign_task(id: string, task: string) -> void
rename_speaker(id: string, key: string, name: string) -> void   // rewrites labels in transcript.md + meta
get_settings() -> Settings                   set_settings(s: Settings) -> void
```
```ts
// src/lib/ipc.ts
export type Status = "recorded" | "queued" | "processing" | "ready" | "failed";
export interface RecordingRow { id: string; title: string; task: string | null; created: string; durationS: number; mode: "meeting" | "in_person"; status: Status; suggestedTask: string | null; }
export interface RecordingDetail extends RecordingRow { transcriptMd: string; summaryMd: string; speakers: Record<string, string>; error: string | null; }
export interface Settings { storageRoot: string; llmBaseUrl: string; llmModel: string; tierOverride: string | null; processWhenIdle: boolean; }
export const api = { listRecordings: () => invoke<RecordingRow[]>("list_recordings"), /* ...one wrapper per command... */ };
```

- [ ] **Step 1: Failing Rust tests** — call command handler fns directly (not through IPC) against a tempdir store: `rename_speaker` rewrites `**Speaker 1:**` → `**Jamie:**` in transcript.md and meta; `assign_task` moves dir and index row; `process_now` flips status to `queued`.
- [ ] **Step 2: Run** — compile fail. Expected.
- [ ] **Step 3: Implement** — handlers are thin: parse args, call storage/index/queue, serialize camelCase via `#[serde(rename_all = "camelCase")]`. Register all in `tauri::Builder`. Settings persisted as JSON in `tauri::api::path::app_config_dir`.
- [ ] **Step 4: Verify** — `cargo test commands` → all passed; `pnpm build` typechecks `ipc.ts`.
- [ ] **Step 5: Commit** — `git commit -am "feat: ipc command layer with typed ts client"`

---

### Task 13: Library UI

**Files:**
- Create: `src/components/Sidebar.tsx`, `src/components/RecordingList.tsx`, `src/components/StatusChip.tsx`, `src/components/RecordingDetail.tsx`, `src/components/SearchBar.tsx`
- Modify: `src/App.tsx`
- Test: `src/components/__tests__/library.test.tsx`

**Interfaces:**
- Consumes: `api` from `src/lib/ipc.ts` (mocked in tests via `vi.mock("../lib/ipc")`).
- Produces: the main window per spec §4.4 — sidebar (tasks + Unsorted/All/Recent), list rows with `StatusChip` (colors: recorded=gray, queued=blue, processing=amber pulse, ready=green, failed=red + error tooltip), suggested-task banner with Accept/Change, detail view with editable summary, transcript with clickable speaker names (opens rename input) and timestamps.

- [ ] **Step 1: Failing component tests** — with mocked `api`: renders task list from `listTasks`; a row with `suggestedTask` shows "Suggested: Accounting 302 — Accept" and clicking calls `api.assignTask(id, "Accounting 302")`; failed row shows the error text; clicking a speaker name and typing "Jamie" calls `api.renameSpeaker(id, "spk1", "Jamie")`; search input debounces then calls `api.search`.
- [ ] **Step 2: Run** — `pnpm test` → FAIL. Expected.
- [ ] **Step 3: Implement** — plain React state (no state library — YAGNI), one `useLibrary()` hook owning fetch+refresh, components presentational. Keep the whole thing keyboard-reachable (list arrow-nav, Enter opens detail).
- [ ] **Step 4: Verify** — `pnpm test` → all passed; `pnpm build` clean. Screenshot beat deferred to Plan B's on-Mac e2e (WSL2 has no display), noted in plan-B checklist.
- [ ] **Step 5: Commit** — `git commit -am "feat: library ui with tasks, status chips, suggestion banner, speaker rename"`

---

### Task 14: Background scheduler wiring

**Files:**
- Create: `src-tauri/src/scheduler.rs`
- Modify: `src-tauri/src/lib.rs` (spawn on app setup)

**Interfaces:**
- Consumes: `Queue`, `IdleSource` (Task 4), `PipelineDeps` (Task 11), model manager (Task 10).
- Produces: a `std::thread` loop — every 30 s: if models ready and `idle.ok_to_run()`, `queue.run_one(...)`; emits Tauri events `queue-changed` (UI refresh) and `processing-progress`. `AlwaysIdle` used on Linux; the trait is Plan B's seam for real idle/power detection.

- [ ] **Step 1: Failing test** — scheduler loop factored as `pub fn tick(...) -> RunOutcome` (pure, testable); test that tick with a queued rec + AlwaysIdle runs the closure, and with a `NeverIdle` test impl does not.
- [ ] **Step 2: Run** — compile fail. Expected.
- [ ] **Step 3: Implement** — thread with `park_timeout`; `process_now` unparks it via a shared `Arc<AtomicBool>` + `Thread::unpark`.
- [ ] **Step 4: Verify** — `cargo test scheduler` → passed; full `cargo test` suite green.
- [ ] **Step 5: Commit** — `git commit -am "feat: idle-time background scheduler with process-now override"`

---

### Task 15: EN/ZH engine bake-off harness (spec §8 decision)

**Files:**
- Create: `src-tauri/src/bin/bakeoff.rs`, `docs/superpowers/specs/bakeoff-result.md`

**Interfaces:**
- Consumes: `Transcriber` trait (Task 6), fixture (Task 5).
- Produces: a decision document — which ASR engine ships as the EN/ZH default.

- [ ] **Step 1: Write the harness** — `cargo run --bin bakeoff -- <audio> <whisper-model>`: runs WhisperTranscriber and (if sherpa-rs exposes SenseVoice, add a `SenseVoiceTranscriber` impl of the same trait — ~40 lines) on the same audio; prints per-engine transcript + wall time + a CER/WER-lite score against `fixtures/bilingual.reference.txt` (write the reference text by hand from Task 5's script lines).
- [ ] **Step 2: Run on the fixture with the small-tier models** — record numbers.
- [ ] **Step 3: Write `bakeoff-result.md`** — table of accuracy/speed, one-paragraph decision, update spec §8. If results are close, Whisper wins (one fewer model family to ship). Re-run on the Mac with large-v3-turbo during Plan B before final call.
- [ ] **Step 4: Commit** — `git commit -am "feat: asr bake-off harness and initial en/zh engine decision"`

---

## Plan B pointer (separate plan, written when the Mac arrives ~2026-07-30)

Not tasks — scope registered so nothing is silently dropped (spec §§4.1, 4.2, 4.6 remainder):
capture engine (ScreenCaptureKit dual-track FLAC, Meetily-code vs Swift-sidecar decision), crash-safe flush + disk guard + pause/resume, meeting watcher + "Record this?" prompt, menu bar UX, real `IdleSource` (idle/power), Metal builds of whisper/sherpa + tier auto-detect on real hw, Ollama install flow + Qwen pull with progress UI, screenshot verification of the library UI, DMG packaging, end-to-end: record a real bilingual Zoom call → idle-process → verified transcript+summary.

## Self-review (done at write time)

- **Spec coverage:** §4.3 pipeline → Tasks 6–11; §4.4 UI → Tasks 12–13; §4.5 storage → Task 2; index → Task 3; queue/retry/idle → Tasks 4, 14; §4.6 tiers/downloads → Task 10; §6 golden test → Tasks 5, 11; §8 bake-off → Task 15; §4.1/4.2/rest of 4.6 → Plan B (platform-bound, explicitly registered above).
- **Placeholders:** prompts, schemas, test bodies, and thresholds are concrete; the two knowingly-deferred externals (model URLs/hashes, sherpa-rs exact API) are marked as implementation-time lookups with the authoritative source named — that is a lookup instruction, not a TBD.
- **Type consistency:** `Utterance`, `SpeakerSpan`, `Suggestion`, `Store`, `Index`, `Queue`, command names, and TS types cross-checked across Tasks 6/7/8/9/11/12/13.
