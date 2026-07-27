# Plan B1 branch review — 2026-07-27

Independent reviewer, fresh context, did not write any of the code. Re-ran and
confirmed every claim in `docs/MAP.md`: **217 Rust + 49 frontend tests green,
clippy clean, `pnpm build` clean.** (Frontend is 54 after the task-10 merge that
landed just after the review started.)

**Verdict: FAIL — two IMPORTANT findings, both empirically demonstrated with
temporary tests that were then reverted.** Neither is data loss. Both are in the
capture-lifecycle code whose own docstrings claim this class of bug was already
closed, which is exactly why an independent pass was worth running.

**Nothing here is fixed yet. This is the next session's first task.**

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

## Finding 3 (MINOR) — `Runtime::pump_once` is dead code with a false doc

Its doc says "The pump thread calls exactly this." `pump_until_done` actually
reimplements the same lock-and-pump logic inline. One occurrence in the whole
codebase, no test. Harmless today; a duplicate of a safety-critical loop that
can silently drift.

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
