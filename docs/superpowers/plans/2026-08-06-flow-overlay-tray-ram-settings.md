# Plan: Flow-class dictation, Cluely-class overlay, curated tray, idle RAM, settings refresh

**Date:** 2026-08-06 · **Status:** PROPOSED — awaiting Mr. Brothers' word
**Decisions locked by Mr. Brothers (2026-08-06):** system-wide dictation; overlay = glass-up + live transcript + AI assist + hide-from-share; tray = rich popover informed by competitors; RAM = unload-everything with a timed idle window (Handy-style); settings = competitor audit **and** visual pass; cleanup engine + sequencing delegated to research.

**Research inputs** (7 agents, 2026-08-06): five OSS/product teardowns + two codebase scouts.
Full reports archived in the session scratchpad; load-bearing facts are inlined below with
their sources. Key repos: Handy (MIT — copy freely), OpenSuperWhisper (MIT),
free-cluely (Apache-2.0), tauri-nspanel / window-vibrancy / tauri-plugin-positioner
(permissive). **Patterns-only, never copy code:** VoiceInk (GPL-3), Glass (GPL-3),
Whispering (AGPL-3).

---

## The one paragraph

Five workstreams, sequenced by dependency: **(1) idle RAM** is a contained fix with a
measured 2.2 GB payoff and ships first; **(2) tray popover** and **(3) overlay glass-up**
share the same new windowing infrastructure (tauri-nspanel + vibrancy + macOSPrivateApi)
and ship as one windowing epic; **(4) system-wide dictation** is the headline build and
introduces the streaming-ASR + paste + permissions machinery; **(5) overlay live
transcript + AI assist** reuses dictation's streaming core; **settings** grows with each
phase and gets a final IA + visual pass. Every phase ends on a stated acceptance beat —
machine-checked wherever a machine can check it (RAM footprint, unit/contract tests),
and labeled honestly as a screenshot or live-hardware beat where only hardware can
answer (paste landing in another app, overlay over a real call). No phase claims done
without its beat.

## Measured baseline (this Mac, 2026-08-06)

| Fact | Value | How measured |
|---|---|---|
| Notetaker idle footprint | **2,367 MB** (+ ~188 MB WebKit helpers) | `footprint -p <pid>` |
| — of which ggml Metal weight buffers | 1,550 MB (`VM_ALLOCATE`) | same |
| — ORT arenas + malloc | ~800 MB | same |
| Handy idle (model auto-unloaded) | **119 MB** | `footprint` |
| large-v3-turbo Metal load, warm | **~350 ms** (cold ~0.5–1.5 s) | whisper-cli timings |
| Handy model unload | 15–28 ms | Handy's own log on this Mac |

Root cause (verified in source): `Runtime::start_processing()` (`core/src/runtime.rs`
~1493) loads Whisper 1.6 GB + optional SenseVoice 239 MB + diarizer eagerly at launch and
moves them into the scheduler thread's closure; `stop_scheduler()` exists but is only
ever called by tests. Nothing unloads, ever.

---

## Phase 1 — Idle RAM: models become a cache, not residents

**Lane: STANDARD. Payoff: idle footprint 2.37 GB → ~100–200 MB.**

1. **`ModelCache` as a shared, leased resource** (`core/src/runtime.rs`, new
   `core/src/models/cache.rs`): replace the moved-in `SchedulerModels` with an
   `Arc<ModelCache>` held in `Inner` and cloned into the scheduler thread. Internally:
   `Mutex<Slot>` where `Slot = Unloaded | Loaded { models: Arc<SchedulerModels>,
   last_used: Instant, leases: usize }`. **This is deliberately designed for two
   consumers from day one** — the scheduler loop now, dictation (Phase 4) and live
   transcript (Phase 5) later — because a cache owned privately by the loop would have
   to be re-architected the moment a second thread needs the models:
   - `acquire() -> ModelLease`: loads if unloaded (~0.5–1.5 s, once per batch),
     increments `leases`; the lease's `Drop` decrements and stamps `last_used`.
   - Idle sweep (the scheduler tick is the one sweeper): unload only when
     `leases == 0 && now − last_used > idle_window`. Unload swaps the slot to
     `Unloaded`, dropping the inner `Arc` — a dictation mid-utterance holding a lease
     keeps its clone alive and the memory is reclaimed when the last lease drops, so
     an unload can never yank models out from under a running job, and a held lease
     can never silently pin memory past its own lifetime.
   - This is a plain mutex + refcount, not condvar machinery — but it **is** cross-
     thread lifecycle coordination, and the unit tests must cover the races: unload
     racing acquire, lease held across the idle window, two acquirers during a load.
   `start_processing` keeps only the model *paths* + the existing missing-models check.
2. **Setting** `modelIdleUnload`: `never / afterBatch / 2m / 5m (default) / 15m / 1h`,
   plus a debug-gated `15s` variant (Handy ships exactly this for testability) —
   "afterBatch" = drop when the queue drains, so 3 queued recordings pay one load.
   The idle window and tick interval are injectable in tests so the unload path is
   exercised in milliseconds, not minutes.
3. **The scheduler loop consumes the cache**: `run_one` acquires a lease per job;
   the tick sweep runs the idle check. All trait objects are already `Send + Sync`,
   and `WhisperTranscriber` creates a fresh `WhisperState` per call, so concurrent
   *transcription* across threads is safe; the lease design above is what makes
   concurrent *lifecycle* (load/unload) safe.
4. **Ollama `keep_alive`**: add the field to `ChatRequest` in `core/src/pipeline/llm.rs`
   (verified absent today) — `"10m"` during a batch, `0` on the final call of a batch so
   qwen3:8b (~5–6 GB, out-of-process) evicts instantly instead of 5 minutes later.
5. **Model-state events** (`model-state-changed`: loaded/unloaded, à la Handy) so the UI
   and tray can say "models sleeping" vs "processing".
6. **WebView destroy-on-close is a separate, optional decision** (~190 MB more, costs
   reopen state + a few hundred ms): surface to Mr. Brothers after 1–5 land, don't bundle.

**Acceptance (machine-checked):** a script captures `footprint` (not RSS — footprint
counts compressed pages; RSS-only would let a fake fix through) at three points: before
load, after a processed batch, one tick past the idle window — run with the debug `15s`
window so the whole beat takes seconds, then once with the real default. Third number
within ~50 MB of the first. Unit tests pin the cache decisions (load-on-demand,
lease-vs-unload races, afterBatch semantics, never).

## Phases 2+3 — one windowing epic, two deliveries

Shared infrastructure, built once at the top of Phase 2 because both the tray popover
and the overlay need it: `"app": { "macOSPrivateApi": true }` in `tauri.conf.json`,
`transparent: true` on the new windows, the **tauri-nspanel** plugin (non-activating
panels), and **window-vibrancy** — vibrancy is a *silent no-op* without the two config
flags. The epic lands as two PRs (tray, then overlay) on the same infra branch.

## Phase 2 — Windowing epic part A: the curated tray

**Lane: STANDARD.** Current state (verified): `src-tauri/src/tray.rs` already has
icon states, mutate-in-place menu, `show_menu_on_left_click(false)`, and left-click →
`show_main()` — the exact behavior Mr. Brothers dislikes.

Design (the hybrid every first-class app ships — and the Windows 11 convention):

- **Right-click → native menu** (evolve current, keep mutate-in-place):
  status line ⸻ Record meeting / Record in person / Pause / Stop ⸻ ★ Highlight moment ⸻
  Open Notetaker ⸻ Settings… / Quit. ("Copy last transcript" is deliberately *not*
  here — no backing feature exists yet; it arrives in Phase 4 with dictation history,
  where "last" has a definition.)
- **Left-click → anchored popover panel (~360×420)**, a third webview window:
  - Idle: big Record split-button (meeting / in-person), 3–5 recent notes
    (click → open note), inline mic picker, "models sleeping/ready" line (Phase 1
    events), footer: gear → Settings, Open Notetaker.
  - Recording: elapsed + level meter, Pause / Stop / ★ Highlight, current app name.
  - (Future slot, not this phase: next-meeting card — needs calendar integration we
    don't have.)
- **Mechanics:** `tauri-plugin-positioner` v2 (`tray-icon` feature,
  `TrayCenter`/`TrayBottomCenter`; **must** forward `on_tray_event` or position
  resolves top-left), window `visible:false, decorations:false, transparent:true,
  alwaysOnTop:true, skipTaskbar:true`; hide on `Focused(false)` + `CloseRequested`.
  macOS: convert to non-activating NSPanel via **tauri-nspanel** (reference:
  `ahkohd/tauri-macos-menubar-app-example`, branch `v2-popover`) so opening it never
  steals focus from the frontmost app. Windows: `TrayBottomCenter` flyout — **NSPanel
  has no Windows equivalent**, so the flyout takes focus like every native Windows
  tray flyout does (Wi-Fi, battery); that is the platform convention, not a defect,
  and hide-on-blur is what makes it feel right.
- Icon polish: `icon_as_template(true)` for idle/paused on macOS, non-template red
  variant while recording; Windows light/dark via `SystemUsesLightTheme`.
- Reuses the existing "two dumb remotes, one owner" event architecture — panel emits
  the same intent events (`tray-record`, …) the menu already emits; `App.tsx`'s single
  listener effect stays the owner.

**Acceptance:** screenshots of panel (idle + recording) on macOS against the design;
`PrintWindow` shots on the PC later per MAP item 11. Tests pin the intent-event wiring.

## Phase 3 — Windowing epic part B: overlay glass-up

**Lane: STANDARD.** Current state (verified): `"overlay"` window built in Rust
(`lib.rs:435–461`) — frameless, always-on-top, `content_protected(true)`,
visible-on-all-workspaces, 300×48, top-right; React `Overlay.tsx` pill with
prompt/recording modes, star/pause/stop.

1. **Glass treatment** (infra from the epic preamble): `apply_vibrancy` with
   `NSVisualEffectMaterial::HudWindow`, radius ~16; `apply_acrylic`/`mica` on Windows.
   Fallback/portable skin: Glass-style pure-CSS glass (layered gradients over
   `rgba(0,0,0,.6)`, full border-radius) — one look on both OSes; decide by
   side-by-side screenshot.
2. **NSPanel conversion** via tauri-nspanel: non-activating (buttons clickable without
   yanking focus from Zoom), `PanelLevel` raised toward screen-saver level +
   `fullScreenAuxiliary` collection behavior → shows over full-screen meetings.
   Known sharp edge: set the style mask through the plugin, not hand-cast objc2
   (crash history, tauri-nspanel #19).
3. **Pill ↔ expanded panel**: start with the simple model — one panel that animates
   `set_size` between 300×48 and ~420×560 (Glass's multi-window children are the
   fallback if resize animation feels cheap). Expanded content this phase:
   elapsed + waveform/level, ★ starred-moments list (timestamped, click-to-jot a note),
   quick text jot into `notes.md`, app name, honest status line.
4. **Hide-from-screen-share — honest scoping.** We already set
   `content_protected(true)`. That is **reliable on Windows**
   (`WDA_EXCLUDEFROMCAPTURE`) and **broken on macOS 15.4+** — ScreenCaptureKit ignores
   `NSWindow.sharingType = .none`; Apple DTS: "no public APIs for preventing screen
   capture" (Tauri #14200 open; Cluely itself hedges this in its docs). Ship it as a
   Settings toggle with truthful copy ("hidden from screen share on Windows; on recent
   macOS, meeting apps may still capture it"). **No unconditional "invisible on Zoom"
   claim, ever.**
5. Polish pass on the prompt mode ("Record Zoom?") with the same glass treatment.

**Acceptance:** screenshots dark+light, pill and expanded, over a full-screen app;
a focus test (overlay click must not deactivate the frontmost app — manual beat with
a screen recording); existing overlay intent-event tests extended.

## Phase 4 — System-wide dictation (the Wispr Flow build)

**Lane: CRITICAL** (it synthesizes keystrokes into other apps).
Architecture follows Handy (MIT, same stack) with OpenSuperWhisper's macOS details.

1. **Hotkey**: `tauri-plugin-global-shortcut` (Carbon-backed — immune to Secure Input,
   no Accessibility needed for the hotkey itself; already a dependency). One handler
   fed by press+release, `pushToTalk: bool` choosing PTT (start on press / stop on
   release) vs toggle — Handy's `shortcut/handler.rs` shape. Default binding
   `CommandOrControl+Alt+D` (rebindable; `fn`-key support via the `handy-keys` crate is
   a later opt-in — it drags in CGEventTap + Secure-Input handling). Escape-to-cancel
   registered only while dictating.
2. **Capture**: `CaptureSources::mic()` → `Box<dyn AudioSource>` accumulating into an
   in-memory `Vec<f32>` — **bypasses `Session` entirely** (no folder, no WAV, no disk
   guard). Verified seam: `MicSource` already delivers 16 kHz mono f32 with
   start/stop/release. Two cautions from the scout, both handled: dictation gets its own
   state slot (never collides with `capture_status`), and `poll_meetings`' mic-hot
   suppression extends to dictation-in-progress.
3. **VAD trim**: Silero through sherpa-onnx (already shipped — no new dep) with
   Handy-style onset/hangover smoothing; gates the level meter and trims silence so
   Whisper never sees dead air.
4. **ASR**: on release/toggle-off, transcribe through a **`ModelCache` lease** from
   Phase 1 (`spans = &[]` — Whisper self-segments; the trait seam is verified). Fresh
   `WhisperState` per run. Vocab biasing via `initial_prompt` from the user dictionary.
   `acquire()` is called **on key-press** so the load overlaps with speaking (Handy's
   trick) — with Phase 1's warm ~350 ms this makes even a cold start invisible; the
   lease is dropped when the paste lands, so the idle timer, not the dictation path,
   decides when memory comes back.
5. **Text insertion (macOS), two tiers**:
   - **v1 — clipboard-borrow paste**: full-fidelity NSPasteboard snapshot via objc2
     (every item, every UTI — text-only snapshots destroy copied images; reimplement,
     Whispering's is AGPL), write transcript with `org.nspasteboard.ConcealedType` +
     `TransientType` (clipboard managers skip it), CGEvent Cmd-V with **layout-aware
     keycode** (OpenSuperWhisper, MIT — Dvorak/"QWERTY ⌘" bug), restore guarded by
     `changeCount` (never a bare timer).
   - **v2 — Handy's receipt-sequenced promise paste** (MIT, copy near-verbatim):
     pasteboard promise = read receipt; eliminates the restore race entirely.
   - No Accessibility grant → **degrade honestly**: leave text on clipboard + notify
     ("copied — press ⌘V"), never fail silently.
   - Windows: enigo paste + clipboard restore (simpler; no Secure Input equivalent).
6. **Cleanup engine — the research recommendation** (Mr. Brothers delegated this):
   **layered, all local.**
   - Layer 0 (always, ~0 ms): deterministic — strip Whisper's `[BLANK_AUDIO]`-class
     markers, spoken commands ("new line", "scratch that") as regex before any LLM.
   - Layer 1 (default ON, budget ≤ ~700 ms): Ollama cleanup pass — filler removal,
     punctuation repair, Backtrack-style self-correction ("at 2 actually 3" → "at 3") —
     through the existing `LlmClient` with `keep_alive` warm during a dictation session.
     **Use a small model for this** (qwen3:1.7b / llama3.2:3b class — pick by measured
     latency on this Mac), *not* qwen3:8b; summaries keep the big model. Short
     utterances (< ~8 words) skip the LLM entirely — raw Whisper punctuation is fine
     and latency stays sub-second.
   - Layer 2 (later): per-app tone presets (frontmost bundle ID → prompt variant) —
     Wispr's Flow Styles, fully local. Explicitly out of v1.
   - Wispr's own docs confirm nobody streams cleaned text — batch-then-paste is the
     correct model; don't chase streaming.
7. **UI**: the Phase 3 overlay gains a dictation state (Flow-bar style: waveform,
   cancel/stop, bottom-center default position for this mode) — same window, new mode
   in `overlay-sync`.
8. **Permissions onboarding**: sequential cards (mic → Accessibility → Input Monitoring
   if needed), each deep-linking the exact System Settings pane; a re-verify check after
   updates. The stable "Notetaker Local Signing" identity (2026-08-05) was verified to
   make **Screen Recording** grants survive rebuilds; that it does the same for
   **Accessibility** is the expected TCC behavior but *unverified* — verifying it on a
   rebuild is an explicit early Phase 4 step, not an assumption. Remember the standing
   lesson: **a missing grant looks like absent data, not an error** — verify by arrival.
9. **History**: every dictation lands in a lightweight local history (text + optional
   audio, off by default); this is what backs "Copy last transcript" in the tray menu
   (added here, not Phase 2 — "last" = most recent dictation). Storage under the
   existing app-dir contract. **Retention is a decision point for Mr. Brothers**: the
   "nothing ever deletes a recording" ground rule protects *recordings*; dictation
   history is a new artifact class, and unbounded audio history grows without limit.
   Proposed default: keep text forever, audio off; if audio is enabled, a visible
   user-set cap with the trade-off stated — surfaced at build time, not decided here.
10. **Command wiring** follows the fixed 4-file pattern (`runtime::COMMANDS`, dispatch
    arm, `#[tauri::command]` wrapper, `ipc.ts`) — the drift test already enforces it.

**Acceptance:** machine-checked — unit tests for VAD gating, command regexes, cleanup
prompt contract, clipboard snapshot/restore round-trip, and a latency harness that
reports release→text timings against the targets (≤ 1.5 s with cleanup, ≤ 0.8 s
without). Live-hardware beat (labeled as such — no machine can press a hotkey over
Slack): hotkey in TextEdit/Slack/browser → speech lands cleaned at the cursor;
clipboard restored including an image; Accessibility-denied path shows the honest
fallback; Accessibility grant survives a rebuild under the stable signing identity.

## Phase 5 — Overlay live transcript + AI assist

**Lane: STANDARD→CRITICAL.** Builds on Phase 4's streaming-ish core.

1. **Live transcript**: tee capture samples (mic = "me"; system audio = "them" — already
   verified working via ScreenCaptureKit on this Mac) into VAD-chunked incremental
   Whisper passes; emit `{speaker, text, isPartial, isFinal}` events; the expanded
   overlay renders Glass's partial-merge pattern (mutate last partial per speaker,
   freeze on final — reimplemented, Glass is GPL). This is *chunked batch*, not true
   streaming — set expectations: lines appear a few seconds behind speech.
2. **AI assist**: an Ask box in the expanded overlay — question + rolling transcript
   context → Ollama `/api/chat` streamed, rendered incrementally (streaming-markdown;
   `smd` is MIT). Local-only, so unlike Cluely there is no privacy story to apologize
   for.
3. Echo cancellation (mic re-capturing system audio) is a known hazard —
   `webrtc-audio-processing` bindings if it bites; evaluate on real audio first.

**Acceptance:** machine-checked — unit tests on the chunking/partial-merge logic and a
contract test that the live path never mutates pipeline inputs. Live-hardware beat
(labeled as such): a real recorded meeting on this Mac with the overlay open —
transcript lines appear during capture, speakers split me/them, one Ask answered from
context; processing pipeline output unchanged.

## Phase 6 — Settings: audit + IA + visual pass

**Lane: STANDARD.** Each phase above already added its settings. This phase closes the
competitor-audit gaps and reorganizes.

- **Target IA** (from the 12-section checklist, trimmed to us): General · Shortcuts
  (all bindables incl. dictation, one page, conflict detection) · Audio (device priority
  à la Krisp, level meter/mic test) · Models & AI (per-task model, download manager,
  **keep-loaded duration** from Phase 1, Ollama picker, cleanup model) · Dictation
  (dictionary, replacements, PTT/toggle, paste behavior, per-app later) · Overlay
  (mode, position, style, hide-from-share toggle with honest copy) · Meetings
  (auto-record policies — exists) · Storage & Privacy (root, keep-audio, logs) ·
  Updates.
- **Two pro-feel options research singled out**: Krisp's Auto/Best-Quality/CPU-Optimized
  performance mode (maps onto our tier override + battery gating — we already have the
  plumbing) and superwhisper's model keep-loaded duration (Phase 1 delivers it).
- **Visual pass**: same glass/token language as the overlay work; search-within-settings
  if section count grows past ~8.
- **Constraint (verified):** ~forty tests pin the BEM markup in `panels.css` /
  `settings.test.tsx`. The visual pass either keeps the class vocabulary or budgets the
  test rewrite explicitly — no silent test deletion.

**Acceptance:** checklist diff (every audit item either present, deliberately declined,
or ticketed); screenshots dark+light; settings tests green (rewritten if markup moved).

---

## Sequencing & effort (my call, as delegated)

| # | Phase | Size | Depends on | Ships value |
|---|---|---|---|---|
| 1 | Idle RAM | S (days) | — | −2.2 GB idle, model events |
| 2 | Tray popover | M | nspanel/vibrancy infra | the tray he asked for |
| 3 | Overlay glass-up | M | same infra as 2 | Cluely look + expand |
| 4 | Dictation | L (the big one) | 1 (shared models), 3 (overlay UI) | Wispr Flow replacement |
| 5 | Live transcript + assist | M–L | 4 (streaming core) | Cluely function |
| 6 | Settings audit + visual | M | all (their settings exist) | coherence |

Phases 2+3 are one epic (shared infra, do the infra once). Each phase lands as its own
branch/PR with its acceptance beat verified before the next starts; MAP updated per
phase. Windows parity: Phases 1, 2, 4 (paste tier), 6 apply directly; overlay vibrancy
degrades to acrylic/CSS; hide-from-share is *better* on Windows.

## Risks worth saying out loud

1. **macOS 15.4+ screen-share hiding is not achievable via public API.** Scoped honestly
   in Phase 3. Anyone promising otherwise is lying to him.
2. **Dictation latency budget** hinges on the small-model cleanup pass — measure on this
   Mac before locking the default; the bypass-for-short-utterances rule is the safety
   valve.
3. **tauri-nspanel style-mask edge** has crash history — use the plugin path, test on
   entry.
4. **Accessibility grants are per-signed-binary** — the stable signing identity
   (2026-08-05) must be used for every dev build that touches paste, or grants evaporate
   and it looks like the code broke.
5. **The 82 pre-existing failing frontend tests** (MAP item 9) make `pnpm test` useless
   as a gate — fixing that `shell is undefined` cause is a prerequisite chore for any
   phase that wants frontend acceptance, i.e. it lands with Phase 2.
