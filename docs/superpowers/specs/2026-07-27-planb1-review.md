# Plan B1 branch review — 2026-07-27

Independent reviewer, fresh context, did not write any of the code. Re-ran and
confirmed every claim in `docs/MAP.md`: **217 Rust + 49 frontend tests green,
clippy clean, `pnpm build` clean.** (Frontend is 54 after the task-10 merge that
landed just after the review started.)

**Verdict: FAIL — two IMPORTANT findings, both empirically demonstrated with
temporary tests that were then reverted.** Neither is data loss. Both are in the
capture-lifecycle code whose own docstrings claim this class of bug was already
closed, which is exactly why an independent pass was worth running.

**All three findings are fixed** (2026-07-27, same day). Each fix is pinned by a
test that was proven to bite: the fix was reverted, the test was watched to fail,
the fix restored. What was done is recorded under each finding below, and the
whole branch is back to green — **224 Rust + 56 frontend tests, clippy
`--all-targets` clean, `pnpm build` clean.**

---

## Finding 1 (IMPORTANT) — renaming a recording *while it is still recording*
strands it, and shows the user a raw filesystem error

`capture/session.rs` caches `rec: RecordingRef` with its directory fixed at
`start()`. `runtime.rs::rename_recording` has no guard against the id belonging
to the live session. So renaming a recording that is currently capturing moves
the folder out from under the open session.

Reproduced. `stop_capture()` then returns:

```
Err(writing /tmp/.../Unsorted/2026-07-27 18.18 Lecture/meta.json
Caused by: No such file or directory (os error 2))
```

Two problems in one:

1. **That raw OS error reaches the user.** It violates the project's own ground
   rule that every message a user can hit is written for someone who is not an
   engineer.
2. **The documented retry does not actually work here.** `Session::stop` promises
   "calling stop again retries exactly that", and does — in isolation. But
   `Inner::finish_session` has already taken the `Session` out of its mutex slot
   and drops it on the `?`, so there is no object left to retry against. A second
   `stop_capture()` finds the slot empty and falls back to `last_recording`,
   which was never set for this id (that line runs *after* the failed stop) — so
   it says "nothing is being recorded right now" about a recording that exists
   and is stuck.

**Not permanent loss:** the audio is intact under the renamed folder, and an app
restart recovers it correctly via `recover_orphans` (the reviewer confirmed
this). Until then the recording is invisible to processing and shows 0:00.

**How a user reaches it:** the library list shows a live capture with status
`Recorded`, indistinguishable from a finished one — there is no "in progress"
status. Any list refresh during a background recording exposes the rename
affordance on it.

**Fix directions (pick at fix time, don't assume):** refuse a rename whose id is
the live session; or teach the session to follow its directory; or add a
distinct in-progress status so the UI never offers the affordance. The
user-facing message must be plain English either way.

### FIXED — refuse the move, and make the retry real

Took the first direction, and deliberately not the third: a disabled button is
not a guarantee, the UI is not the only caller, and hiding the affordance would
have left the hole open to anything else that invokes the command.

1. `Inner::refuse_while_capturing` rejects any command that would move the live
   recording's folder, in a sentence that says what to do — *"this recording is
   still being recorded, so it cannot be renamed yet — stop the recording first,
   then try again."* Only the live session is protected; a recording that has
   stopped capturing but is still encoding has closed files and is safe to move.
2. **`assign_task` had the same bug** and the review did not catch it — filing a
   recording under a task moves the folder exactly like a rename does. Guarded
   too, with its own test. A guard on half the hole is not a fix.
3. **The retry now works.** `Inner::finish_session` puts the `Session` back in
   its slot when `stop()` fails instead of dropping it on the `?`, so the object
   that `Session::stop` promises can retry still exists to retry with. A second
   Stop closes the recording out properly. `start_capture` in between no longer
   says "a recording is already in progress" — it says *"the last recording has
   not finished saving yet — press Stop again to finish saving it."*

Tests: `renaming_the_live_recording_is_refused_instead_of_stranding_it`,
`filing_the_live_recording_under_a_task_is_refused_for_the_same_reason`,
`renaming_a_recording_that_is_not_the_live_one_still_works_mid_capture`,
`a_stop_that_cannot_save_stays_retryable_instead_of_stranding_the_recording`.
The first asserts on the *absence* of `os error` and `no such file` in the
message, so the raw-error regression cannot come back quietly.

## Finding 2 (IMPORTANT) — `capture_status()` reports idle for the whole FLAC
encode after an auto-stop

The `finishing` lock added earlier covers `stop_capture()` — proven, and the
existing test genuinely fails if the lock is removed. It does **not** cover
`capture_status()` or `list_recordings()`.

Reproduced: with an 8-second fake capture, **433 consecutive polls** showed
`capture_status().state == Idle` while `list_recordings()` still showed the
recording as `Recorded` rather than `Queued` — the entire duration of
`compress_tracks`. This is not a narrow race; it is the whole encode window, and
a real lecture-length encode is longer (the reviewer did not measure that).

Matters most on **auto-stop** (disk guard, dead mic), which is the path the
record bar polls rather than a user pressing Stop. The UI re-enables Start
before the previous recording is queued and indexed. No data-loss path was
found — a second recording gets its own directory — but the user can start
recording again while the disk-guard message is still being acted on, and the
library can briefly show the just-stopped recording as stuck.

### FIXED — the window has a name now: `CaptureState::Finishing`

Calling that stretch "idle" was the bug, so it stopped being called that.

- New `CaptureState::Finishing` (`"finishing"` in `ipc.ts`), covering the whole
  span from the last sample to the recording being queued and indexed — both
  halves of it: a session still in its slot with capture over, and a session
  already taken out and being encoded. `Inner::closing` publishes the second
  half, written while the session lock is still held so no poll can land in the
  gap between the two and read "idle". A `ClearOnDrop` guard clears it however
  the close-out leaves — including a `?` or a panic — because a stuck "Saving…"
  is a record bar that never re-arms.
- `capture_status()` still never blocks: it reads the published window rather
  than waiting on the `finishing` lock. A poll that waited out a FLAC encode
  would freeze the window it is drawing.
- A **live** session outranks a closing one, so starting the next lecture while
  the last is still encoding is allowed and shows the right recording. That is
  deliberate: blocking Start through an hour-long lecture's encode would be a
  worse bug than the one being fixed.
- The record bar shows *"Saving your recording — it will appear in the library
  in a moment,"* disables Start and Pause, and **keeps Stop live** so a save
  that failed can be retried by pressing it again.

Tests: `capture_never_reads_idle_before_the_recording_is_in_the_queue` asserts
the invariant the UI actually depends on — *the first moment capture reads idle,
the recording is already in the queue* — driven through the auto-stop path with
nobody pressing Stop. Plus
`a_recording_still_being_put_away_reports_itself_and_its_length`,
`a_live_recording_outranks_one_that_is_still_being_put_away`, and two frontend
tests covering the banner and the poll-driven recording → saving → idle
transition.

## Finding 3 (MINOR) — `Runtime::pump_once` is dead code with a false doc

Its doc says "The pump thread calls exactly this." `pump_until_done` actually
reimplements the same lock-and-pump logic inline. One occurrence in the whole
codebase, no test. Harmless today; a duplicate of a safety-critical loop that
can silently drift.

### FIXED — one loop, and the doc is true

The lock-and-pump step moved to `Inner::pump_once`. `Runtime::pump_once` is a
one-line delegation to it and `pump_until_done` calls it directly, so there is
no second copy left to drift. Covered by every existing capture test, all of
which now run through that one function.

## What the review confirmed as genuinely solid

- Every Plan B1 done-criterion met, checked individually.
- FLAC verify-before-delete confirmed by mutation.
- Carry-overs I3 (index on completion) and I4 (scheduler wake) tested end to end.
- Tests read closely in `session.rs`, `flac.rs`, `recover.rs`, `runtime.rs`
  assert real on-disk sample counts and status transitions, not `is_ok()`.
  No test found passing for the wrong reason in those modules.

## Edges of this review — what was NOT checked

Stated plainly so the next pass knows where the dark corners are:

- No mutation-testing pass on `ollama/`, `power/`, or `watch/` tests for the
  passes-for-the-wrong-reason failure mode. Only `capture/` and `runtime.rs`.
- `ollama/mod.rs` NDJSON/timeout logic not examined in depth.
- `pipeline/` modules not touched (out of the review's scope).
- Contract drift checked for command and argument *names* only. **Return-type
  shapes are not covered by the automated drift test at all.** `CaptureStatus`
  and `CaptureState` were spot-checked by hand and are fine; `PullProgress`,
  `OllamaStatus`, `MeetingEvent` and `Settings` were not checked field by field.
- Finding 2 not measured against a real lecture-length recording.

## Still open after the fixes

- **Finding 2 is still unmeasured at lecture length.** The fix makes the window
  *reported* rather than *shorter*; how long "Saving…" actually sits on screen
  after an hour of audio is a B2 question, on the Mac, with real FLAC timings.
- **The library list still shows a live capture as `Recorded`**, because there
  is no on-disk in-progress status. Renaming or filing it is now refused in
  plain English, which is what made it a bug; the cosmetic half would cost a new
  `Status` variant in the on-disk contract and was judged not worth it today.
- The dark corners listed above (no mutation pass on `ollama/`, `power/`,
  `watch/`; return-type shapes not covered by the drift test) are untouched by
  this round.
