# Update — 2026-08-05, late session: the permission is FIXED, capture proven in-app

**Root cause of the "granted 25 times and still refused" loop:** TCC binds a
grant to the app's code signature. Ad-hoc signing (`signingIdentity: "-"`)
pins the identity to the build's cdhash — every rebuild silently orphaned the
grant while the Settings checkbox stayed on. Builds before 21:08 additionally
had an *invalid* signature no grant could attach to at all.

**The fix, in place and verified:** a self-signed certificate
(**"Notetaker Local Signing"**) in a dedicated keychain
`~/Library/Keychains/notetaker-sign.keychain-db` (its password is a generic
password named `notetaker-sign-keychain` in the login keychain — CI-style,
because Mr. Brothers' login-keychain password did not match his login
password, so `set-key-partition-list` on the login keychain was impossible).
The installed app is re-signed with it; designated requirement is now
`certificate leaf = H"cc31effe…"` — **stable across rebuilds, so the grant can
never be orphaned again**. `tauri.macos.conf.json` now signs future builds
with the same identity. Stale TCC entries were cleared with
`tccutil reset ScreenCapture com.georgebrothers.notetaker`.

**Proven on hardware, by Mr. Brothers himself (21:48):** a 40 s meeting
recording through the app UI — mic track audible on playback, and the first
non-empty `audio-system.flac` in this app's life. In-app log shows
`system audio: first buffer read, 960 samples`.

**Not a bug after all:** the global hotkey (Cmd+Opt+N) worked the whole time —
every "dead" press had actually called `start_capture`, opened the mic, then
hit the orphaned permission and rolled back silently.

## New problems found tonight, in priority order

1. **Processing fails with `error: "diarization"`, 5 attempts,** on the 21:48
   recording. Meeting mode diarizes the *system* track; this one is ~40 s of
   pure silence (nothing was playing). Prime suspect: diarizing a silent/empty
   signal has never been exercised on any platform. Being reproduced through
   `notetaker-serve` with stderr visible. `meta.error` carries only the stage
   name — the pipeline swallows the real error text; that is itself a bug.
2. **The macOS window chrome is Windows chrome** (Mr. Brothers, 21:55): the
   custom titlebar draws Windows-style minimize/maximize/close glyphs, on the
   right. On a Mac they must be traffic lights, on the left — likely the
   overhaul's `feat(native): the window draws its own titlebar` needs a
   per-OS branch or native decorations on macOS.

---

# Handover — 2026-08-05 (macOS bring-up)

Read `docs/MAP.md` first — especially **"The Mac day"** and **"The signature,
and the silence"**. This file is only what is in flight right now.

## Where the work is

Branch **`claude/notetaker-mac-support-907380`**, rebased onto
`origin/claude/app-ui-ux-overhaul-96e4c6` (the UI overhaul, 55 commits ahead of
`main`). **Nothing has been pushed.** `main` is *not* the base — building on it
was a mistake caught early and corrected by rebase.

Commits, oldest first:
1. `feat(macos)` — ScreenCaptureKit system audio, Metal, Info.plist, dylibs
2. `fix(macos)` — the two-pass `AudioBufferList`, plus the capture examples
3. `docs` — the Mac day
4. `fix(macos)` — bundle signing, entitlements, first-buffer permission check
5. `docs` — a missing permission looks like absent data

## State: the Mac works

Verified on this machine, not reasoned about:

- Builds all four crates natively (~30 s cold). **535 Rust tests pass, clippy
  clean at `-D warnings`**, frontend typechecks.
- **Metal is live** — `GPU name: Apple M5 Pro`, `Metal total size = 1623.92 MB`
  for `large-v3-turbo`. No Xcode required.
- **Both capture paths produce real audio.** `--example system-audio`: 79,146
  samples, peak 0.2966. `--example microphone`: 79,487 samples, "MacBook Pro
  Microphone".
- **A full meeting recording through the real `Session` path works**:
  `audio-mic.flac` + `audio-system.flac` (8.08 s, 16 kHz mono, valid FLAC), with
  `system audio: first buffer read, 960 samples` in the log.
- `.app` and `.dmg` build, properly ad-hoc signed
  (`Identifier=com.georgebrothers.notetaker`, Info.plist bound, hardened
  runtime, `codesign --verify --deep` valid), all dylibs resolving from
  `Contents/Frameworks`.

## The one thing not yet proven

**A recording inside `Notetaker.app` itself, with sound actually audible.**

- The app's Screen Recording grant could not be tested end to end because
  **macOS output is muted** (`output muted: true`, volume 69). Every system
  track captured so far is therefore *valid but silent*. Unmute, then record.
- Mr. Brothers says he has now granted permission to the app. `start_capture`
  has still never appeared in the app's log, so **no recording has yet been
  attempted through the app UI** — only through `notetaker-serve`.
- I cannot drive the app: `osascript` is refused ("not allowed to send
  keystrokes") because Accessibility is not granted to the terminal. Either he
  presses Record, or he grants Accessibility and the hotkey
  `Cmd+Option+N` can be sent.

## Traps that already cost time — do not repeat

- **Two app directories.** The Tauri shell uses `app.path().app_data_dir()` →
  `~/Library/Application Support/com.georgebrothers.notetaker`;
  `notetaker-serve` uses `paths::default_app_dir()` →
  `~/Library/Application Support/Notetaker`. Different logs, index and settings.
  **When debugging the app, read the bundle-id one.** (Next item 2.)
- **Rebuild the binary you are about to test.** A Session test was run against a
  `notetaker-serve` built *before* the two-pass fix; its 44-byte
  `audio-system.wav` was misread as a capture bug that had already been fixed.
- **A missing permission looks like absent data, not an error.** ScreenCaptureKit
  reports success and simply never calls back.
- `pnpm tauri build` always ends with `Error A public key has been found, but no
  private key` (updater signing, `TAURI_SIGNING_PRIVATE_KEY` unset). **The
  `.app` and `.dmg` are already written when this fires** — it is not a failed
  build.
- Homebrew rustup lives at `/opt/homebrew/opt/rustup/bin`, **not** `~/.cargo/bin`.

## Known-bad, pre-existing, not mine

**82 frontend tests fail** in 3 files on the UI-overhaul branch — `shell` is
undefined at `capture.test.tsx:475`, so `shell.handlers.clear()` throws in
`beforeEach`. Confirmed by running them on the untouched branch. `pnpm test` is
useless as a gate until fixed. (Next item 9.)

## Housekeeping owed

Two throwaway recordings I created are sitting in his library and should
probably go — **not deleted without his word**:
`~/Notetaker/Unsorted/2026-08-05 21.01 Mac end-to-end capture test` and
`2026-08-05 21.06 system audio diagnostic`. His real one,
`2026-08-05 19.39 Zoom meeting`, has an intact 5 MB mic track and a 0-byte
system track — keep it.

---

# Handover — 2026-08-01

Read `docs/MAP.md` first; this file only covers what is in flight right now.

## 2026-08-01 continuation — C1 is ready for Windows use

**PR [#2](https://github.com/George-Brothers/notetaker/pull/2)** is open from
`claude/previous-session-continuation-ea389c` to `plan-b-capture`. It is pushed
at `c202671`; every CI job is green (Linux, macOS, Windows native tests, and the
Windows installer plus its bundled-library check).

This continuation completed the C1 work that had been planned but not yet
landed:

- every recording with real audio has a Listen route; a verified FLAC remains a
  success even if its source WAV cannot be deleted, and the app records that
  durable capture note;
- durable app-data logging exists, and `get_settings`, `ollama_status`, and
  `detected_tier` log their entry and elapsed exit time so Settings freezes can
  be diagnosed from a real machine;
- an installed but stopped Ollama is told to start rather than being told to
  download; recordings queued for absent speech models say exactly that; and
  the first-run screen promises that the explicit speech-model button is the
  only download trigger;
- first run makes a bounded scan of recognized locations for an existing speech
  model. A size match is only offered; clicking **Use it instead** SHA-256
  verifies and copies it, never moves or changes the source file.

Local verification before the PR: 395 core Rust tests, Clippy with warnings as
errors, 145 frontend tests, the frontend production build, and the cross-target
platform check. CI repeated the complete gate on all three target OSes.

### Still required: one interactive Windows truth pass

The green CI artifact was downloaded to
`C:\Users\georg\Downloads\Notetaker-C1` (both MSI and NSIS installers). There
was no existing Notetaker installation. This WSL session cannot start either
installer: the NSIS launch returned **Access is denied**, and Windows Installer
returned **could not create installation log** in that same folder. Neither
attempt created an install directory. Treat this as a WSL-to-Windows bridge
blocker, not an application result.

From an interactive Windows desktop, run the NSIS installer in that folder and
then verify: Settings opens responsively and emits the new timings to the log;
the real Ollama state is described truthfully; a fresh recording yields FLAC or
the explicit retained-WAV note; model-less queued recordings say they are
waiting; and an already-downloaded speech model is offered for safe adoption.
Record what actually happens before changing code again.

## Where the branches are

- **`main`** — the real project, as of 2026-07-30. Plan A, B and C plus the
  Windows installer.
- **`claude/cross-platform-mac-pc-web-e3af6f`** — everything: Plan B, Plan C,
  the Windows installer. **PR #1 was merged by Mr. Brothers on 2026-07-30**, so
  `main` is finally the real project rather than a spec. Note it merged at an
  *older* tip than the branch reached — the last three commits (the honest
  setup state and its docs) were not in it, and arrived on `main` through the
  branch below instead. The branch can be deleted; nothing on it is unique.
- **`claude/real-use-fixes`** — *current*, and the only branch that matters.
  Branched on 2026-07-30 at his instruction, because a 100-commit branch is not
  a review unit. `main` is merged into it, so it sits **4 commits ahead and 0
  behind**: a PR from it to `main` is a genuinely small, readable diff. All new
  work goes here.

Everything is pushed. Tree clean. CI green on all three platforms.

## The state that matters

**The app is built, installed and running on Mr. Brothers' own Windows PC**, and
he has used it. That closed two things at once: the installer works, and
**capture works on real hardware** — three recordings, mic audio, meeting and
in-person modes, FLAC written. Both were assumptions for the entire life of this
project.

His verdict on using it, verbatim where it matters, is the work list below.

## What he asked for, and what is done

He answered four questions on 2026-07-30. His answers are the spec.

| # | Ask | State |
|---|---|---|
| 1 | Never block; be honest about what won't work | **Done**, shipped |
| 2 | Playback on every recording, processed or not | **Done** |
| 3 | Settings — all four gaps | Not started |
| 4 | Detect existing Ollama, let him pick the model | Not started |
| 5 | Processing automatic + manual button | Design already correct; only honesty was missing (#1) |

**His governing rule for all of it**, on how hard to push setup:

> dont force it also check if someone already has ollama and allow them to
> choose their own models for things like note making from transcripts. i just
> want the app to be like okay fine but just so u know it wont work

No modals, no blocking, no disabled buttons. Say the true thing once and get out
of the way.

### 1. Honest setup state — DONE

`Runtime::setup_status` (`core/src/runtime.rs`) reads the **disk and the live
scheduler**, not session memory. Returns `transcribing`, `missing`,
`downloadBytes`, `waiting`, `tier`. Infallible on purpose — a status check that
can fail leaves the UI with nothing to say. `missing_models()` is shared with
`start_processing` so the two cannot drift.

`ModelSpec` gained `label` and `bytes`. Every size was read from the real URL's
`content-length` on 2026-07-30: 1.6 GB large-v3-turbo, 190 MB small-q5_1,
239 MB SenseVoice, 33 MB the two diarization files. **Display only** — the
sha256 does the job that matters, so a drifted number can mislabel but never let
a bad file through.

UI: `src/components/SetupNotice.tsx`, plus `processNow` in `App.tsx`. The wording
is the entire feature, so `SetupNotice.test.ts` tests the wording.

### 2. Playback on every recording

**Done.** The player itself was never broken: it lived below
`TranscriptPanel`'s no-segments guard, exactly as this handover suspected. An
unprocessed recording therefore had audio on disk but no route to play it.
`NoteView` now owns the single player, which opens from the Listen toolbar
control (or automatically on the Transcript tab); the transcript consumes that
same player for timestamp seeking and active-line highlighting.

The work also found a second cause of apparent silence. `audio_tracks` accepted
a 44-byte, header-only WAV as playable, so the player chose that empty system
track by default. It now accepts FLACs with bytes and WAVs with frames, which
keeps genuinely quiet audio playable while leaving a no-sample WAV out of the
chooser.

### 3. Settings — he picked all four

1. **Nothing is pre-filled.** Settings should open showing what the app already
   chose, not blanks.
2. **No microphone picker.** New platform work: enumerate cpal input devices,
   show which is in use, let him change it. `notetaker-platform` must keep
   depending on no other notetaker crate — that property is what lets Linux
   cross-check the Windows and macOS code at all.
3. **No model status.** Downloaded or not, size, re-download, switch tier.
   `setup_status` already returns most of this.
4. **Storage location** neither visible nor changeable, and no open-folder
   button.

### 4. Ollama

Detect an install that is already there instead of pushing a download, list the
models he already has, and let him choose which writes summaries. There is
already an `ollama/` module with `ollama_status`, `pull_model` and
`pull_progress`. Note the standing caveat: **Ollama is verified against
`httpmock` only** — the NDJSON field names come from knowledge of the API, not
observed traffic. He has a real Ollama; that is now checkable.

### 5. Two capture bugs, diagnosed but not chased

Both found in his actual files, both **silent**, which is what makes them
serious:

- `audio-system.flac` is **0 bytes on all three** recordings. WASAPI loopback
  produced nothing. **Open question for him, asked and not yet answered: was
  anything actually playing through his speakers?** If not, this is expected
  and chasing it is hunting a ghost. Do not assume either way.
- Two of three kept `audio-mic.wav` **next to** the `.flac`. Per the ground
  rules that means FLAC verification did not confirm — but `meta.error` is
  `null`, so nothing was reported. The silence is a bug whatever the cause.

## How to work on this

Build environment, the WSL→Windows technique, and the verification table are all
in `docs/MAP.md`. The two things most easily forgotten:

- **`pnpm build` is the only typecheck.** vitest does not typecheck, so a
  contract change can pass every test and still be broken. Run both.
- **`scripts/check-platforms.sh` before pushing.** A CI round trip is ten
  minutes; that script reproduces most macOS/Windows failures in thirty seconds.

Adding a command touches four layers and drift tests enforce every one:
`COMMANDS` in `runtime.rs` → a match arm and the test list in `dispatch.rs` →
a `#[tauri::command]` wrapper and `generate_handler!` in `src-tauri/src/lib.rs`
→ `src/lib/ipc.ts`. Miss one and a named test tells you which.

## Open, waiting on Mr. Brothers

1. **Was anything playing through his speakers during those recordings?**
   Decides whether the 0-byte system track is a bug or a non-event. Asked, not
   yet answered — do not guess either way.
2. **Signing.** The installer is unsigned, so SmartScreen warns once. Fine for
   one user, a wall for anyone else. Costs money.

Merging PR #1 was on this list and is done.

## The one sentence that matters

Everything he found in ten minutes of real use was invisible to 488 passing
tests, because none of it is a *failure* — it is the app being silent. Weight
the next round of work accordingly.
