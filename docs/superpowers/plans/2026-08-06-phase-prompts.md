# Per-phase agent prompts — Flow / overlay / tray / RAM / settings build

**Date:** 2026-08-06 · **Spec:** `docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md`
**Dispatch model:** one fresh agent per phase (Phases 2+3 = one agent, two PRs).
Run strictly in order — every prompt assumes the previous phase's branch is merged
locally. Paste one block per session, verbatim.

Why per-phase and not one mega-run: each phase ends on an acceptance beat that must be
independently verified before the next phase builds on it; a fresh context per phase
prevents long-run drift; a failed phase costs one run, not six.

---

## PROMPT 1 — Phase 1: Idle RAM (models become a leased cache)

```
You are working in the Notetaker repo (Tauri v2 desktop app; Rust workspace under
src-tauri/ with crates core/, platform/, server/, plus the app crate; React/TS UI
under src/). You are implementing exactly one phase of an approved plan.

READ FIRST, in this order:
1. docs/MAP.md — the repo map and ground rules. Obey every ground rule.
2. docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md — your spec.
   Your scope is "Phase 1 — Idle RAM" plus the "Measured baseline" table. Nothing else.

MISSION: Notetaker idles at a measured 2,367 MB because Runtime::start_processing()
(core/src/runtime.rs, ~line 1493) loads Whisper (1.6 GB ggml-large-v3-turbo on Metal),
optionally SenseVoice (239 MB), and the sherpa diarizer eagerly at launch and moves
them into the scheduler thread's closure; stop_scheduler() exists but is only called
by tests. Fix: models become an Arc<ModelCache> with a lease API and a timed idle
unload, exactly as specified in Phase 1 of the plan (Mutex<Slot> with a leases count;
acquire() -> ModelLease whose Drop decrements and stamps last_used; the scheduler tick
is the one idle sweeper; unload only when leases == 0 and the idle window has passed).
Reference implementation for the policy: Handy (github.com/cjpais/Handy, MIT — you may
copy its patterns and code with attribution), src-tauri/src/managers/transcription.rs
idle-watcher. Measured on this machine: warm reload of the big model is ~350 ms, so
unloading is nearly free.

DELIVERABLES (all from the plan's Phase 1 section):
1. ModelCache + lease in a new core/src/models/cache.rs, wired through runtime.rs and
   scheduler.rs. start_processing keeps only model paths + the existing missing-models
   check (Processing::ModelsMissing behavior must not change).
2. Settings field modelIdleUnload: never / afterBatch / 2m / 5m (default) / 15m / 1h,
   plus a debug-gated 15s variant. Idle window and tick are injectable in tests.
   Follow the existing Settings pattern in core/src/api.rs (serde defaults, camelCase
   rename) so old settings.json files still parse.
3. keep_alive added to ChatRequest in core/src/pipeline/llm.rs (currently absent):
   "10m" during a batch, 0 on the batch's final call.
4. model-state-changed events on load/unload so the UI can show models sleeping/ready.
5. An acceptance script (scripts/ or a documented command) that captures `footprint -p
   <pid>` (NOT ps RSS — footprint counts compressed pages; RSS lets a fake fix pass)
   before load, after a processed batch, and one tick past the idle window. Pass =
   third number within ~50 MB of the first. Run it with the 15s debug window.

REQUIRED TESTS: unit tests for the cache races the plan names — unload racing acquire,
lease held across the idle window, two acquirers during a load — plus load-on-demand,
afterBatch semantics, and never. Run the full suite from src-tauri/:
cargo test -p notetaker-core -p notetaker-platform -p notetaker-server
and scripts/check-platforms.sh (clippy cross-check for Windows; the app crate only
compiles in CI, and a drift test in core reads src-tauri/src/lib.rs).

HARD RULES:
- Nothing ever deletes a recording. Nothing rewrites notes.md. Capture reports
  Finishing, never Idle, until the recording has landed.
- The unload must never yank models from under a running job (lease design guarantees
  this — test it).
- Work on a new branch off the current one. Commit locally. DO NOT push, merge, or
  open a PR — report done with the footprint numbers and test output instead.
- If a check fails twice, stop and report state; do not loop silently.
- Report honestly: if a number was not measured, say so. Never claim the acceptance
  beat passed without pasting the three footprint numbers.
```

---

## PROMPT 2 — Phases 2+3: windowing epic (curated tray popover, then Cluely-grade overlay)

```
You are working in the Notetaker repo (Tauri v2; Rust under src-tauri/, React/TS under
src/). You are implementing one two-part epic of an approved plan, as two PRs on one
infra branch.

READ FIRST, in this order:
1. docs/MAP.md — repo map and ground rules. Obey every ground rule.
2. docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md — your spec:
   the "Phases 2+3 — one windowing epic" preamble, "Phase 2", and "Phase 3" sections.
3. src-tauri/src/tray.rs (172 lines), src-tauri/src/lib.rs lines ~420-521 (overlay
   window creation, close-to-tray), src/components/Overlay.tsx, and the tray/overlay
   wiring in src/App.tsx (~lines 142-459) — the existing architecture is "two dumb
   remotes, one owner": tray and pill emit intent events; App.tsx's single listener
   effect owns all state. PRESERVE this architecture; the panel is a third remote.

PREREQUISITE CHORE (do first, its own commit): 82 frontend tests fail from one cause —
`shell` is undefined at src/components/__tests__/capture.test.tsx:475, breaking
beforeEach in three files. Fix that root cause so pnpm test is a usable gate. Also run
pnpm build — it is the only typecheck (vitest does not typecheck).

SHARED INFRA (once, before either part): "app": { "macOSPrivateApi": true } in
tauri.conf.json; transparent: true on the two new/changed windows; add the
tauri-nspanel plugin (github.com/ahkohd/tauri-nspanel, branch v2.1 for Tauri v2) and
window-vibrancy. Vibrancy is a SILENT NO-OP without macOSPrivateApi + transparent +
CSS body{background:transparent}. Known sharp edge: set the non-activating style mask
through the plugin, never hand-cast objc2 (crash history, tauri-nspanel issue #19).

PR 1 — TRAY (plan Phase 2):
- Left-click no longer opens the main window. It opens an anchored popover panel
  (~360x420, a third webview window): idle state = Record split-button (meeting /
  in-person), 3-5 recent notes (click opens that note in the main window), inline mic
  picker, models sleeping/ready line (listen for model-state-changed from Phase 1);
  recording state = elapsed + level meter, Pause / Stop / Star highlight, current app
  name; footer = gear -> Settings, Open Notetaker.
- Positioning via tauri-plugin-positioner v2 with the tray-icon feature
  (TrayCenter on macOS / TrayBottomCenter on Windows). CRITICAL: forward every tray
  event with tauri_plugin_positioner::on_tray_event or position resolves top-left.
- Hide on Focused(false) and CloseRequested. macOS: convert the panel to a
  non-activating NSPanel (reference: ahkohd/tauri-macos-menubar-app-example, branch
  v2-popover) so it never steals focus. Windows: the flyout takes focus — that is the
  platform convention (Wi-Fi/battery flyouts do), not a defect.
- Right-click menu (evolve tray.rs, keep the mutate-in-place approach — its comment
  explains why rebuilds are avoided): status line / Record meeting / Record in person /
  Pause / Stop / Star highlight moment / Open Notetaker / Settings / Quit. Do NOT add
  "Copy last transcript" — it has no backing feature until Phase 4.
- Icons: icon_as_template(true) for idle/paused on macOS, non-template red variant
  while recording; Windows light/dark via the SystemUsesLightTheme registry key.
- Panel emits the SAME intent events the menu already emits (tray-record, etc.).

PR 2 — OVERLAY (plan Phase 3):
- Convert the existing "overlay" window (built in Rust, lib.rs ~435-461) to a
  non-activating NSPanel; raise its level toward screen-saver and add the
  fullScreenAuxiliary collection behavior so it shows over full-screen meetings.
- Glass look: apply_vibrancy(HudWindow, radius ~16) on macOS, acrylic/mica on Windows;
  build the pure-CSS glass fallback too (layered gradients over rgba(0,0,0,.6), full
  border-radius) and pick by side-by-side screenshot.
- Pill <-> expanded panel: one window animating set_size between 300x48 and ~420x560.
  Expanded content: elapsed + level, starred-moments list (timestamped), quick text
  jot appended to notes.md (NEVER rewrite notes.md — append only), app name, honest
  status line. Keep every existing intent event working.
- Hide-from-screen-share: content_protected(true) is already set. It works on Windows
  and macOS <= 15.3 and is IGNORED by ScreenCaptureKit on macOS 15.4+ (Apple removed
  the API; Tauri issue #14200). Ship it as a Settings toggle whose copy says exactly
  that. Never claim unconditional invisibility.
- Polish the "Record {app}?" prompt mode with the same glass treatment.

LICENSES: Glass (pickle-com/glass) is GPL-3 — read for patterns, NEVER copy its code.
Handy is MIT, free-cluely is Apache-2.0 — copying allowed with attribution (NOTICE
file exists). tauri-nspanel / window-vibrancy / positioner are permissive.

ACCEPTANCE: pnpm test green (including the fixed 82), pnpm build clean, cargo test +
scripts/check-platforms.sh clean. Screenshots: panel idle + recording, overlay pill +
expanded, dark + light, overlay over a full-screen app. Manual beat (label it as such
in your report): clicking the overlay or panel must not deactivate the frontmost app
on macOS. Windows verification is deferred to the MAP's standing Windows truth pass.

HARD RULES: no user-facing string may say "your Mac" (app ships on PC too). Every
color comes from tokens in src/styles/theme.css — no hardcoded colors. Work on a new
branch; commit locally per PR; DO NOT push, merge, or open PRs on the remote — report
done with screenshots and test output. If a check fails twice, stop and report state.
```

---

## PROMPT 3 — Phase 4: system-wide dictation (the Wispr Flow build)

```
You are working in the Notetaker repo (Tauri v2; Rust under src-tauri/, React/TS under
src/). You are implementing the highest-stakes phase of an approved plan: system-wide
dictation — hold a hotkey anywhere on the OS, speak, and cleaned text lands at the
cursor of whatever app is focused. This phase synthesizes keystrokes into other
people's apps: treat it as CRITICAL. Slow is fine; silent failure is not.

READ FIRST, in this order:
1. docs/MAP.md — repo map and ground rules.
2. docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md — your spec is
   "Phase 4 — System-wide dictation", all 10 numbered items. Follow them exactly;
   they encode decisions already made and adversarially reviewed.
3. core/src/runtime.rs, core/src/models/cache.rs (the Phase 1 ModelCache + lease you
   will consume), core/src/pipeline/transcribe.rs (the Transcriber trait — spans=&[]
   is your seam), platform/src/mic.rs (MicSource already delivers 16 kHz mono f32),
   src/hooks/useGlobalHotkeys.ts (existing hotkey registration pattern).

REFERENCE IMPLEMENTATIONS AND LICENSES (from the plan's research):
- Handy (github.com/cjpais/Handy) — MIT, same Tauri+Rust stack. COPY FREELY with
  attribution: its shortcut/handler.rs press+release PTT/toggle shape, and later its
  paste_tx/macos.rs receipt-sequenced paste (tier 2).
- OpenSuperWhisper (github.com/Starmel/OpenSuperWhisper) — MIT: the layout-aware
  Cmd-V keycode resolution (Dvorak / "QWERTY cmd" layouts type the wrong key if you
  synthesize 'v' naively).
- VoiceInk (GPL-3), Whispering/epicenter (AGPL-3), Glass (GPL-3): patterns only,
  NEVER copy their code. Whispering's clipboard.rs full-pasteboard snapshot/restore
  is the best design — reimplement it yourself in ~120 lines of objc2.

BUILD, in this order (details per plan item):
1. Hotkey via tauri-plugin-global-shortcut (already a dependency; Carbon-backed =
   immune to Secure Input, no Accessibility needed for the hotkey itself). One handler
   for press+release; pushToTalk bool chooses hold vs toggle. Default
   CommandOrControl+Alt+D, rebindable via the existing HotkeyField pattern.
   Escape-to-cancel registered only while dictating.
2. Capture: CaptureSources::mic() accumulating into an in-memory Vec<f32>. Bypass
   Session entirely (no folder, no WAV, no disk guard). Dictation gets its OWN state
   slot — it must never collide with capture_status or block a real recording — and
   poll_meetings' mic-hot suppression (runtime.rs ~1109) must extend to dictation.
3. VAD: Silero through sherpa-onnx (already shipped) with onset/hangover smoothing;
   gates the level meter, trims silence.
4. ASR: acquire() a ModelCache lease ON KEY-PRESS (load overlaps with speech; warm
   load is ~350 ms); transcribe with spans=&[] on release; fresh WhisperState per run;
   initial_prompt from the user dictionary; drop the lease when the paste lands.
5. Paste, macOS tier 1: full NSPasteboard snapshot via objc2 (every item, every UTI —
   text-only snapshots destroy copied images); write transcript with
   org.nspasteboard.ConcealedType + TransientType markers; CGEvent Cmd-V with
   layout-aware keycode; restore guarded by changeCount — NEVER a bare timer.
   No Accessibility grant -> leave text on clipboard + notify ("copied — press
   Cmd-V"); never fail silently. Windows: enigo paste + guarded restore.
6. Cleanup, layered and all-local: Layer 0 always (regex: strip [BLANK_AUDIO]-class
   markers, spoken commands "new line" / "scratch that"); Layer 1 default-on (Ollama
   pass — filler removal, punctuation, self-correction "at 2 actually 3" -> "at 3")
   through core/src/pipeline/llm.rs's LlmClient with keep_alive warm during a session.
   Benchmark small models (qwen3:1.7b / llama3.2:3b class) on this machine and pick by
   measured latency — NOT qwen3:8b. Utterances under ~8 words skip the LLM. Per-app
   tone (Layer 2) is explicitly OUT of this phase.
7. UI: the overlay window gains a dictation mode in overlay-sync (Flow-bar style:
   waveform, cancel/stop, bottom-center for this mode).
8. Permissions onboarding: sequential cards (mic -> Accessibility -> Input Monitoring
   if needed), each deep-linking the exact System Settings pane; re-verify after
   updates. The stable "Notetaker Local Signing" identity is verified to preserve
   Screen Recording grants across rebuilds; VERIFY EARLY that it does the same for
   Accessibility (expected TCC behavior, unverified) — a rebuilt binary silently
   losing the grant looks like broken code. A missing grant looks like absent data,
   not an error: detect by arrival, never assume.
9. History: dictations land in local history (text; audio off by default); this backs
   a new "Copy last transcript" tray item (last = most recent dictation). Retention
   is a flagged decision for the repo owner — implement the proposed default (text
   kept, audio off) and surface the cap question in your report; do NOT invent an
   auto-delete.
10. Command wiring follows the repo's fixed 4-file pattern: runtime::COMMANDS table,
    dispatch.rs arm, #[tauri::command] wrapper in src-tauri/src/lib.rs, invoke in
    src/lib/ipc.ts — a contract test fails if any side drifts. The app crate only
    compiles in CI; scripts/check-platforms.sh is your local cross-check.

ACCEPTANCE — machine-checked: unit tests for VAD gating, command regexes, cleanup
prompt contract, clipboard snapshot/restore round-trip; a latency harness reporting
release->text against targets (<=1.5 s with cleanup, <=0.8 s without). Live-hardware
beat (label it as such): dictate into TextEdit, Slack, and a browser — cleaned text at
the cursor; clipboard restored including an image; Accessibility-denied path shows the
honest fallback; grant survives a rebuild. cargo test + pnpm test + pnpm build clean.

HARD RULES: no cloud calls, ever — everything local. Nothing deletes a recording;
notes.md is never rewritten. Work on a new branch; commit locally; DO NOT push, merge,
or open a PR — report done with latency numbers and the beat results. Two failed
attempts at anything -> stop and report state; never loop silently.
```

---

## PROMPT 4 — Phase 5: overlay live transcript + AI assist

```
You are working in the Notetaker repo (Tauri v2; Rust under src-tauri/, React/TS under
src/). You are implementing one phase of an approved plan: the expanded overlay gains
a live meeting transcript and a local-AI Ask box.

READ FIRST, in this order:
1. docs/MAP.md — repo map and ground rules.
2. docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md — your spec is
   "Phase 5 — Overlay live transcript + AI assist".
3. The Phase 4 dictation code (VAD-chunked transcription path, ModelCache leases) —
   you are reusing its streaming core, not building a second one.
4. core/src/capture/session.rs + runtime.rs pump loop — you will tee samples from the
   live capture; platform/src/macos/speaker.rs (system audio, verified working on this
   Mac via ScreenCaptureKit).

BUILD:
1. Live transcript: tee capture samples (mic = "me", system audio = "them") into
   VAD-chunked incremental Whisper passes through a ModelCache lease held for the
   session. Emit {speaker, text, isPartial, isFinal} events to the overlay. This is
   chunked batch, not true streaming — lines appear a few seconds behind speech, and
   the UI copy should not pretend otherwise.
2. Rendering: the partial-merge pattern — keep the last partial message per speaker,
   mutate it in place as new partials arrive, freeze on isFinal. (This pattern is
   described in the plan; Glass's implementation is GPL-3 — reimplement, never copy.)
   Auto-scroll only when already near the bottom.
3. Ask box: question + rolling transcript context -> Ollama /api/chat with
   stream: true, tokens rendered incrementally (streaming-markdown; the `smd` JS lib
   is MIT and safe to use). All local — no cloud, ever.
4. THE LIVE PATH IS READ-ONLY over the capture samples. It must not mutate, delay, or
   drop anything the recording pipeline writes — a contract test must prove pipeline
   output is byte-identical with the live path on and off.
5. Echo (mic re-capturing system audio) is a known hazard: evaluate on real audio
   first; if it bites, webrtc-audio-processing bindings are the named remedy. Do not
   add the dependency speculatively.

ACCEPTANCE — machine-checked: unit tests on chunking + partial-merge; the read-only
contract test above; cargo test + pnpm test + pnpm build clean. Live-hardware beat
(label it as such): a real recorded meeting on this Mac with the overlay open —
transcript lines appear during capture, speakers split me/them, one Ask answered from
context, and the recording's processed transcript/summary come out unchanged.

HARD RULES: nothing deletes a recording; notes.md never rewritten; meeting mode still
refuses to start when system audio is unavailable (that contract is load-bearing).
New branch; commit locally; DO NOT push, merge, or open a PR — report done with the
beat results. Two failed attempts -> stop and report state.
```

---

## PROMPT 5 — Phase 6: settings audit + IA + visual pass

```
You are working in the Notetaker repo (Tauri v2; Rust under src-tauri/, React/TS under
src/). You are implementing the final phase of an approved plan: the Settings surface
gets a completeness audit against best-in-class competitors, a reorganized IA, and a
visual pass.

READ FIRST, in this order:
1. docs/MAP.md — repo map and ground rules.
2. docs/superpowers/plans/2026-08-06-flow-overlay-tray-ram-settings.md — your spec is
   "Phase 6 — Settings" including its target IA and the 12-section checklist
   references.
3. src/components/Settings.tsx (982 lines, six sections), src/styles/panels.css
   (READ ITS HEADER COMMENT), src/components/__tests__/settings.test.tsx.

THE ONE CONSTRAINT THAT BITES: ~forty tests are pinned to the BEM markup
(.settings-*, .status-chip*, .progress-bar*) documented in panels.css. Either keep
that class vocabulary through the redesign, or rewrite the tests deliberately and say
so in your report — a diff that silently deletes settings tests is a failed task.

BUILD:
1. Reorganize into the target IA from the plan: General / Shortcuts (every bindable
   action on one page, conflict detection — HotkeyField already exists) / Audio
   (device priority, level meter + mic test) / Models & AI (per-task model, download
   manager, keep-loaded duration from Phase 1, Ollama picker, cleanup model from
   Phase 4) / Dictation (dictionary, replacements, PTT vs toggle, paste behavior) /
   Overlay (mode, position, style, hide-from-share toggle — its copy must state
   honestly that macOS 15.4+ may not hide it) / Meetings (existing auto-record
   policies) / Storage & Privacy / Updates.
2. Audit pass: walk the plan's checklist; every item ends present, deliberately
   declined (say why), or listed as ticketed in your report. The two named "pro feel"
   options to include: a Krisp-style Auto / Best-Quality / CPU-Optimized performance
   mode (maps onto the existing tier override + battery gating plumbing) and the
   model keep-loaded duration.
3. Visual pass: same token/glass language as the overlay work. Every color from
   src/styles/theme.css tokens — hardcoding one silently breaks dark mode. Add
   search-within-settings if sections exceed ~8.
4. Settings persistence follows the existing pattern (core/src/api.rs Settings struct,
   serde defaults so old settings.json files parse; camelCase mirror in
   src/lib/ipc.ts).

ACCEPTANCE: checklist diff in the report (present / declined / ticketed for every
item); screenshots of every section, dark + light; pnpm test green (rewritten tests
included and named), pnpm build clean, cargo test clean.

HARD RULES: no user-facing string says "your Mac"; every message is written for
someone who is not an engineer. New branch; commit locally; DO NOT push, merge, or
open a PR — report done with screenshots and the checklist diff. Two failed attempts
-> stop and report state.
```
