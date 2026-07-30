# Handover — 2026-07-30

Read `docs/MAP.md` first; this file only covers what is in flight right now.

## Where the branches are

- **`main`** — one commit, the Plan A spec. Effectively empty.
- **`claude/cross-platform-mac-pc-web-e3af6f`** — everything: Plan B, Plan C,
  the Windows installer. 100 commits, carried by **PR #1**, which has never
  merged. An attempt to merge it on 2026-07-30 was **blocked by the permission
  classifier**; that was surfaced rather than worked around, and it is still
  open. Mr. Brothers can merge it himself with
  `gh pr merge 1 --merge --repo George-Brothers/notetaker`.
- **`claude/real-use-fixes`** — *current*. Branched off the above on
  2026-07-30 at his instruction, because a 100-commit branch is not a review
  unit. New work goes here. A PR from it can target the big branch (small
  diff, reviewable) or `main` once #1 lands.

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
| 2 | Playback on every recording, processed or not | Not started |
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

He was explicit: a player at the top of **any** recording, whether or not it has
been transcribed, so raw audio is listenable the second recording stops. He also
liked a play button on each library row.

There *is* a working player — it was verified in a browser against the served UI,
click-to-seek and line highlighting and all. The bug is almost certainly that it
is reachable only through the transcript view, so an unprocessed recording has no
route to it. **Confirm that before building anything**; it may be a five-line fix
rather than a feature. `audio_path` is already a wired command and
`lib/transport.ts` already owns `audioSrc`, which is the one place the desktop
and served transports genuinely differ.

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

1. **Merge PR #1**, or say to leave it. The merge is blocked for this agent.
2. **Was anything playing during those recordings?** Decides whether the
   0-byte system track is a bug or a non-event.
3. **Signing.** The installer is unsigned, so SmartScreen warns once. Fine for
   one user, a wall for anyone else. Costs money.

## The one sentence that matters

Everything he found in ten minutes of real use was invisible to 488 passing
tests, because none of it is a *failure* — it is the app being silent. Weight
the next round of work accordingly.
