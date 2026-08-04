# UI/UX Overhaul — "The same room, lit from within"

**Date:** 2026-08-04 · **Approved by Mr. Brothers** (design pitch artifact, plates 01–07,
plus four alignment picks) · **Base:** `claude/previous-session-continuation-ea389c`
@ `561ecb0`, fast-forwarded into branch `claude/app-ui-ux-overhaul-96e4c6`.

This spec is written so that **inexpensive models can execute it without making
a single taste decision**. Every color, string, size, and default in this
document is final. If a value is not in this spec or the plan, the executor
does not invent it — they stop and report.

## Decisions of record (his picks, 2026-08-04)

| Decision | His pick |
|---|---|
| Look & feel | One identity, two materials: dark = **luminous glass** (hero), light = **porcelain paper**. Same accent both modes. |
| Light-mode paper | **Porcelain** (cool near-neutral), not warm cream. |
| App icon / mascot | **A — "Echo," the ghost-wave** (waveform mouth, aurora glow). |
| Accent | **Violet→cyan aurora** (`#8B72FF → #4DD6FF` dark, `#6C4FF0 → #0E7FA8` light). |
| Tray | **True tray app**: close hides, state in icon, quick actions, autostart toggle. |
| Build approach | **Layered overhaul**: re-skin tested components, rebuild Settings/sort/palette, add native layer. |
| Execution constraint | Spec + plan must be **paint-by-numbers** for cheaper models. |

## 1. Identity

- **Echo** is the product mark: a rounded ghost whose mouth is a 3-bar
  waveform. Echo appears in exactly three places: the app icon, the tray, and
  empty states. Echo is never animated in v1.
- **Glow is meaning.** Only *live* things glow: the recording pulse, the level
  meter, the audio playhead, the active row's accent edge, a hotkey capture in
  progress. Static chrome never glows. This is the single rule that keeps the
  app "sleek" instead of "RGB gamer."
- **The aurora gradient** (`--grad-aurora`) appears only on: primary action
  buttons, the playhead fill, the level meter, and Echo. Never on text except
  the first-run greeting title.

## 2. Design tokens — the single source of truth

The whole UI already flows through CSS custom properties in
`src/styles/theme.css` (verified: zero hardcoded colors elsewhere; one neutral
overlay in `panels.css` stays). The re-skin is therefore **a values change plus
additive tokens** — existing token *names* must not be renamed or removed.
Tailwind utility bridging (`@theme inline`) stays as-is, extended for new tokens.

### 2.1 Dark — "luminous glass" (default when OS is dark)

| Token | Value | | Token | Value |
|---|---|---|---|---|
| `--c-app` | `#0B0B12` | | `--c-accent` | `#8B72FF` |
| `--c-raised` | `#14141F` | | `--c-accent-hover` | `#9D87FF` |
| `--c-sunken` | `#07070C` | | `--c-accent-fg` | `#0B0B12` |
| `--c-hover` | `#1C1C2B` | | `--c-accent-soft` | `#241F45` |
| `--c-selected` | `#232338` | | `--c-accent-2` *(new)* | `#4DD6FF` |
| `--c-border` | `#232336` | | `--c-accent-2-soft` *(new)* | `#10333F` |
| `--c-border-strong` | `#34344E` | | `--c-recording` | `#FF5C51` |
| `--c-fg` | `#EDEDF7` | | `--c-recording-soft` | `#3A1815` |
| `--c-fg-ai` | `#A9A9C5` | | `--c-warn` / `-soft` | `#F5B84D` / `#2E2412` |
| `--c-fg-muted` | `#8A8AA8` | | `--c-error` / `-soft` | `#FF6B61` / `#3A1815` |
| `--c-fg-faint` | `#62627E` | | `--c-ok` / `-soft` | `#3ED9A4` / `#10352A` |

Speakers: `--c-spk-1 #4DD6FF`, `--c-spk-2 #B78CFF`, `--c-spk-3 #FFB86B`,
`--c-spk-4 #63E6BE`, `--c-spk-5 #FF8FB8`.
Shadows: `--shadow-card: 0 1px 2px rgb(0 0 0 / .5), 0 4px 16px rgb(0 0 0 / .35)`;
`--shadow-pop: 0 8px 24px rgb(0 0 0 / .55), 0 24px 64px rgb(0 0 0 / .6)`.
Glows *(new)*: `--glow-accent: 0 0 20px rgb(139 114 255 / .35)`;
`--glow-recording: 0 0 16px rgb(255 92 81 / .45)`.

### 2.2 Light — "porcelain" (default when OS is light)

| Token | Value | | Token | Value |
|---|---|---|---|---|
| `--c-app` | `#F7F7FA` | | `--c-accent` | `#6C4FF0` |
| `--c-raised` | `#FFFFFF` | | `--c-accent-hover` | `#5B3FE0` |
| `--c-sunken` | `#EFEFF5` | | `--c-accent-fg` | `#FFFFFF` |
| `--c-hover` | `#E9E9F1` | | `--c-accent-soft` | `#EEEAFE` |
| `--c-selected` | `#E7E4FA` | | `--c-accent-2` *(new)* | `#0E7FA8` |
| `--c-border` | `#E3E3EC` | | `--c-accent-2-soft` *(new)* | `#E3F4FB` |
| `--c-border-strong` | `#CFCFDD` | | `--c-recording` | `#E0342B` |
| `--c-fg` | `#17171F` | | `--c-recording-soft` | `#FCE9E7` |
| `--c-fg-ai` | `#5D5B75` | | `--c-warn` / `-soft` | `#B07514` / `#FBF1DD` |
| `--c-fg-muted` | `#79778E` | | `--c-error` / `-soft` | `#C92F26` / `#FBE9E7` |
| `--c-fg-faint` | `#A3A1B8` | | `--c-ok` / `-soft` | `#1F9D6C` / `#E3F5EE` |

Speakers: `#0E7FA8`, `#7C4FD8`, `#B26A1B`, `#1E8F68`, `#C24A7E`.
Shadows: `--shadow-card: 0 1px 2px rgb(23 23 31 / .05), 0 4px 14px rgb(108 79 240 / .06)`;
`--shadow-pop: 0 6px 18px rgb(23 23 31 / .10), 0 20px 48px rgb(108 79 240 / .14)`.
Glows: `--glow-accent: 0 3px 14px rgb(108 79 240 / .20)`;
`--glow-recording: 0 2px 12px rgb(224 52 43 / .25)` — **in light mode glow is a
tinted shadow**, same token names so components never branch on theme.

### 2.3 Shared

- `--grad-aurora: linear-gradient(92deg, var(--c-accent), var(--c-accent-2))` *(new)*.
- Radii unchanged: `--radius-card: 0.75rem`, `--radius-control: 0.5rem`.
- Motion *(new)*: `--t-fast: 120ms`, `--t-med: 200ms`, `--t-slow: 280ms`,
  `--ease-swift: cubic-bezier(0.2, 0, 0, 1)`. Overlays enter fade+scale
  (0.98→1, `--t-med`). The existing `prefers-reduced-motion` kill-switch stays.
- The two-layer theme mechanism (OS default, `data-theme` override, full
  repetition of both blocks) is **kept exactly as structured today** — the
  executor changes values inside the existing four blocks, nothing else.
- Type scale: 11px caps-labels (`+0.1em` tracking), 12.5px hints, 13.5px
  secondary, 15px body, 17px section titles, 20px note titles
  (`-0.02em` tracking, weight 700). Durations and timestamps always
  `font-variant-numeric: tabular-nums`. Font stays Inter Variable.

## 3. The shell

- **Custom titlebar** (desktop only, `isDesktop()` guard; the LAN/web build
  keeps no window controls): `decorations: false` in `tauri.conf.json`; the
  existing header becomes the drag region (`data-tauri-drag-region`), with —
  left: record control + live meter; center-left: "Notetaker" wordmark at
  `--c-fg-faint`; right: theme toggle, settings, then minimize / maximize /
  close buttons (44px hit areas; close hover = `--c-recording` background).
  Double-click on empty drag area toggles maximize. Escape hatch: setting
  `decorations` back to `true` restores stock chrome — one line.
- **Sidebar rail:** search field stays; **new sort/filter row** under it
  (§4). Day headers stay. Active row gets a 2px inset accent edge
  (`box-shadow: inset 2px 0 0 var(--c-accent)`).
- Existing two-pane responsive behavior (rail vs note, `md` breakpoint) is
  untouched.

## 4. Library sorting and filtering

- Sort menu options, exact labels and order: **Newest first** (default) ·
  **Oldest first** · **Longest first** · **A to Z**.
- Filter menu options: **Everything** (default) · **Still processing** ·
  **Had a problem** · **Has my notes**.
- Under Newest/Oldest, day grouping stays. Under Longest and A-to-Z the list is
  flat (no day headers) — a duration-sorted list grouped by day is nonsense.
- Persistence: `localStorage` keys `notetaker.librarySort` ∈
  `newest|oldest|longest|alpha` and `notetaker.libraryFilter` ∈
  `all|processing|error|notes`. Applies to every view (per-view memory is
  explicitly out of scope for v1).
- Sorting/filtering is pure frontend (`useLibrary`), no IPC change.

## 5. Settings — from a scroll to a place

Six sections in a left nav (labels exact): **General · Recording · Hotkeys ·
Transcription & AI · Storage · Updates**. Settings opens as a full overlay
with the nav; supports `initialSection` prop for palette deep links. Every
control shows its current value pre-filled; no control ever renders empty
while data exists (load order already fixed upstream).

| Section | Contents (existing → moved, *new*) |
|---|---|
| General | theme (system/light/dark select), *close button hides to tray* (checkbox, default on), *start Notetaker with Windows* (checkbox, default on, via autostart plugin), language checkboxes (moved from "Languages and speech") |
| Recording | *microphone picker* (device list + "System default"), auto-record per-app table (moved), keep-WAV checkbox (moved from "Recording files") |
| Hotkeys | *start/stop recording* recorder row (default `Ctrl+Alt+N`), *show/hide Notetaker* recorder row (default `Ctrl+Alt+Space`) |
| Transcription & AI | model size tier, speech engine select (moved), *model status list with per-model state and re-download*, Ollama status + pull, LLM base URL + model (grouped under an "Advanced" disclosure, collapsed by default) |
| Storage | storage root with **Choose folder…** native picker + current path + *disk usage of the folder*, open-logs button (moved) |
| Updates | version line, check button, download-and-restart (unchanged behavior) |

Hotkey recorder behavior: click row → it enters listening state ("Press the
keys…"), captures one combination, writes it. If OS registration fails, the row
shows, verbatim: **"That combination is taken by another app — pick a
different one."** and keeps the old binding. Registration state is never
silent.

## 6. Command palette — find & jump only

- **Removed rows:** Record a meeting, Record in person, Stop recording, Switch
  to light/dark mode, Ask about this recording.
- **Kept:** recordings (as today).
- **Added groups:** **Tasks** (jump to task view) and **Settings** (deep links:
  General, Recording, Hotkeys, Transcription & AI, Storage, Updates — each
  opens Settings at that section).
- Input placeholder becomes **"Jump to…"**. Footer hint line: full-text search
  lives in the sidebar (kept). Every row keeps/gains its real shortcut hint.
- Recording is reachable exactly two ways: the record control, and the global
  hotkey. The quick theme *toggle* lives only in the titlebar; the
  System/Light/Dark *preference* select lives only in Settings → General (it
  persists via `useTheme`'s existing localStorage mechanism, never the
  settings struct). Cmd/Ctrl+J keeps Ask.

## 7. Native layer (Windows now, macOS wired but executed in B2)

- **Plugins added:** `tauri-plugin-global-shortcut`, `tauri-plugin-autostart`,
  `tauri-plugin-single-instance`, `tauri-plugin-window-state`,
  `tauri-plugin-dialog` (folder picker). Updater/opener/process stay.
- **Tray** (Tauri `TrayIcon` API, no extra plugin): left-click = show/focus
  window (or hide when focused); right-click menu, exact labels top to bottom:
  state row (**"Start recording"** idle / **"Stop recording"** + elapsed while
  live), **"Open Notetaker"**, separator, **"Settings"**, **"Quit
  Notetaker"**. Tooltip: "Notetaker — recording 12:34" pattern while live,
  "Notetaker" otherwise.
- **Tray state icons:** `src-tauri/icons/tray/idle.png`, `recording.png`,
  `paused.png` (32×32; Echo silhouette; recording adds the red dot). Frontend
  drives state via new app-crate command `set_tray_status(state)` called from
  `useCapture` on every state change — dumb, observable, and testable.
- **Close-to-tray:** intercept close → hide. First time only, an in-app dialog
  explains, verbatim: title **"Still running"**, body **"Notetaker keeps
  running here in the tray so meeting detection and your recording hotkey
  still work. Quit completely from the tray icon."**, buttons **"Got it"** /
  **"Quit instead"**. Flag `notetaker.trayExplained` in localStorage.
  While `closeToTray` is off in Settings, close quits (recording-safe: a live
  recording prompts stop-and-save first — reuse existing stop flow).
- **Global hotkeys:** accelerators stored in settings as
  `CommandOrControl+Alt+N` / `CommandOrControl+Alt+Space` (Tauri notation, so
  macOS maps to Cmd automatically). Registered at startup and re-registered on
  change. Toggle-record works with the window hidden; a recording started by
  hotkey shows the tray recording state immediately.
- **Single instance:** second launch focuses the existing window.
  **Window state:** size/position persisted and restored (plugin default).
- **Settings struct additions** (core `Settings`, camelCase over IPC):
  `inputDevice: string | null` (null = system default),
  `hotkeyToggleRecord: string`, `hotkeyShowHide: string`,
  `closeToTray: boolean`. Autostart is **not** in the struct — the plugin owns
  that state; the checkbox reads/writes the plugin API directly.
- **New IPC:** `list_input_devices() → { id, label, isDefault }[]` (cpal
  enumeration) and `set_tray_status(state)`. Both are **app-crate-only
  commands** — they are desktop-shell concerns, are NOT added to
  `runtime::COMMANDS`, and live in a clearly-marked desktop-only block in
  `ipc.ts` behind `isDesktop()` (the LAN/web build hides the mic picker and
  never calls them). The existing core↔ipc contract test stays untouched.

## 8. Echo assets and the icon pipeline

- Sources live in `src-tauri/icons/source/`: `echo.svg` (full-color app icon,
  final art embedded in the plan, tuned eyes + catchlights version),
  `echo-tray-idle.svg`, `echo-tray-recording.svg`, `echo-tray-paused.svg`
  (flat silhouettes: `#B9B7D0` idle, `#EFEFFA` + `#FF5C51` dot recording,
  + `#F5B84D` dot paused).
- Render script `scripts/render-icons.sh`: headless Chromium renders
  `echo.svg` → `echo-1024.png`, then `pnpm tauri icon src-tauri/icons/source/echo-1024.png`
  regenerates the full platform set; tray SVGs render to the three 32×32 PNGs.
  Script is idempotent and CI-runnable.
- `productName` stays "Notetaker"; window title stays "Notetaker".

## 9. Restyle pass over existing components

All remaining components (RecordBar, NoteView, Notepad, PlayerBar,
TranscriptPanel, AskPanel, ActionItems, FirstRun, SetupNotice, MeetingPrompt,
StatusChip, ui.tsx primitives) are restyled **through tokens and the type
scale only** — behavior, props, and test-visible text do not change except
where this spec says so. The playhead and meter adopt `--grad-aurora` +
`--glow-accent`. Primary buttons adopt the aurora gradient with
`--c-accent-fg` text. Empty library state gets Echo (dim, 96px) + the line
**"Nothing here yet — hit record, or press {hotkey}."** where `{hotkey}` is
the configured `hotkeyToggleRecord` formatted for display (desktop) or the
line ends after "record" (web build).

## 10. Verification doctrine (unchanged law, applied per slice)

- Frontend: `pnpm test --run` and `pnpm build` (the only typecheck) both green
  per task. Rust: `cargo test -p notetaker-core` from `src-tauri/`, clippy
  `--all-targets` clean; app crate compiles on Windows CI (not on this box).
- Every visual task ends with a headless-Chromium screenshot compared against
  the pitch (`docs/superpowers/specs/assets/` keeps the approved pitch
  renders); every native task ends with the Windows install + `PrintWindow`
  screenshot loop from the WSL host bridge.
- **Cheap-model guardrails:** never invent a color/string/size not in spec or
  plan; never rename tokens; never edit `src-tauri/core/src/capture/**`,
  `queue/**`, `storage/**`, `index/**`; if a task's check fails twice, stop
  and report rather than improvise; the asked-for change being in the diff is
  done-criterion #1.

## 11. Non-goals and honest limits

- No macOS execution in this effort (tray/menu-bar and hotkeys are wired
  cross-platform but B2 owns the Mac day). No web/LAN restyle risk: tokens
  flow there automatically; tray/hotkey/titlebar are desktop-gated.
- No bulk operations, tags, folders, per-view sort memory, or Echo animation.
- Windows decides whether new tray icons start behind the chevron; pinning is
  a one-time user drag (shown once, not fought).
- Custom titlebar trades away the Windows 11 snap-layout flyout on hover of
  maximize (edge-drag snapping still works). Accepted; reversible by config.

## 12. Risks

| Risk | Containment |
|---|---|
| Global hotkey collisions | Registration errors surface in Settings verbatim; defaults chosen for low collision (`Ctrl+Alt+N/Space`). |
| Custom chrome regressions (drag, maximize, multi-monitor) | Titlebar lands as its own task with an explicit manual checklist on real Windows; `decorations:true` escape hatch. |
| Tray icon invisible at 16px | Silhouette-only variant, verified by screenshot at 100% and 150% DPI. |
| Re-skin breaks a test that asserts a class | Tests assert behavior/text, not colors (audited); any failure is fixed by updating the *test's* selector, never by skipping. |
