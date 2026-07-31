# The app tells the truth on Windows

2026-07-30. Written after running the installed app on Mr. Brothers' own PC and
reading his three real recordings off disk — not from the second-hand work list
in `docs/HANDOVER.md`, which turned out to be wrong on four counts.

## What an hour on the real machine actually found

Every item below was reproduced on his hardware, and every one is a **silence**
rather than a failure. None of it is visible to the 488 passing tests, because
the app never says any of it out loud.

### 1. Settings freezes the app — reproducible

Responsive before the settings gear is clicked, `Not Responding` about thirteen
seconds after, and it never comes back. Confirmed twice on a fresh launch.

Settings mounts three commands and nothing else: `getSettings`, `detectedTier`,
`ollamaStatus`. `detectedTier` is a `sysinfo` memory read and a core count —
fast. `getSettings` reads one small JSON file. `ollamaStatus` makes an HTTP
call. That is the suspect, but **it is a suspect, not a diagnosis** — the
thirteen seconds do not match its three-second `timeout_global`, and guessing
is what produced the handover this spec exists to correct.

This is the whole of his "Settings — nothing is pre-filled" complaint. Nothing
is pre-filled because nothing ever finishes loading.

### 2. There is no log file

`%LOCALAPPDATA%\Notetaker` holds binaries and nothing else. Every `log::warn!`
in the codebase writes to nowhere on Windows.

This is why item 1 has no diagnosis yet, and it is why item 4 below went
unnoticed for a week. **Logging is not a nice-to-have here; it is the
instrument every other item on this list is measured with.** It goes first.

### 3. Ollama is installed, and the app says it is not

His machine:

```
C:\Users\georg\AppData\Local\Programs\Ollama\ollama.exe   (and on PATH)
```

The app's first-run panel: *"Ollama isn't installed. Download it from
https://ollama.com/download …"*

`ollama/mod.rs`:

```rust
const BINARY_NAME: &str = "ollama";
const EXTRA_INSTALL_LOCATIONS: &[&str] = &[
    "/Applications/Ollama.app", "/usr/local/bin/ollama", "/opt/homebrew/bin/ollama",
];
```

On Windows the executable is `ollama.exe`, so `dir.join("ollama")` never
exists; the three fallbacks are macOS paths. `binary_present()` **cannot**
return true on Windows.

His words: *"i DO have ollama its just not running so idk why i would download
it again."* The app had the right state available — `OllamaStatus` already
carries `installed` and `running` separately — and threw it away by getting
`installed` wrong. The fix is the detection *and* the sentence: someone whose
Ollama is installed but stopped needs "start it", never "download it".

### 4. The stray `.wav` is a Windows unlink failure

Two of his three recordings kept both files:

```
17.40 Meeting/   audio-mic.flac 176178   audio-mic.wav 386346
17.44 In person/ audio-mic.flac  46088   audio-mic.wav 108500
```

`finalize_to_flac` (`capture/flac.rs:47-80`) encodes, verifies, and only then
removes the WAV. Both files surviving means the encode **and** the verify
succeeded and `std::fs::remove_file` failed — the capture thread's handle was
still open, which Linux permits and Windows refuses. The function then returns
`Err` on what was actually a success, and `Runtime::compress_tracks` swallows
it into the `log::warn!` that goes nowhere.

Three distinct wrongs: a delete that fails, a success reported as a failure,
and a failure reported to no one.

The handover called this "FLAC verification did not confirm". It confirmed
fine.

### 5. Nothing has ever been transcribed

All three recordings read `Queued`. The first-run panel reads *"Download the
speech models — Not started"*, directly under the sentence *"Happens
automatically in the background — or download now."* It did not happen
automatically.

So the transcription, diarization, and summarization pipeline — the reason this
app exists — has never run on real audio on his machine.

### 6. What the handover got wrong, for the record

- *"`audio-system.flac` is 0 bytes on all three"* — nothing is 0 bytes. One
  recording has a **4,469-byte** system FLAC (quiet, valid, playable); another
  has a **44-byte** system WAV (header, zero frames); the third has no system
  track, correctly, because it was in-person.
- *"FLAC verification did not confirm"* — it confirmed. See item 4.
- *"He has a real Ollama; that is now checkable"* — true, and the app was
  lying about it. See item 3.
- The 0-byte premise reached the playback spec and plan before it was caught.
  Both are corrected.

**The lesson, and it is the same one as last week:** a work list assembled from
symptoms mis-assigns causes. Go and look.

## What this changes about the shape of the work

Mr. Brothers asked for four things after using the app. Reading the machine
splits them differently than the asking did:

| He asked for | What it actually is |
|---|---|
| Settings: nothing is pre-filled | The freeze (item 1). Not a settings feature. |
| Settings: mic picker | Genuinely new work. Unaffected. |
| Settings: model status | Genuinely new work. `setup_status` already has most of it. |
| Settings: storage location | Already built and working — blocked behind the freeze. |
| Ollama: detect an existing install | A one-line platform bug (item 3), plus the sentence. |
| Ollama: let me choose my model | Genuinely new work. `OllamaStatus.models` already returns the list. |

Three of six are bugs wearing a feature's clothes.

## The plan splits in two

These are independent enough to be separate specs and coupled enough to have a
hard ordering.

### Plan C1 — the truth pass (bugs, all of it)

In order, because each one is the instrument for the next:

1. **A log file on Windows** (and macOS). A rolling file in the app data
   directory, plus a way to open it from the UI. Nothing else can be
   diagnosed until this exists.
2. **The Settings freeze.** With logging in place, re-run on the real machine,
   read which command never returns, and fix that. The fix is unknown until
   the log says; the *method* is what this spec commits to, not the patch.
3. **Ollama detection on Windows.** `ollama.exe`, real Windows install
   locations, and a distinct sentence for installed-but-stopped: *"Ollama is
   installed but not running. Open it and summaries will start working."*
4. **The unlink bug.** Retry the delete after the writer's handle is closed;
   treat a verified FLAC as a success whatever the delete does; and when the
   WAV genuinely cannot be removed, write a `capture_note` so he is told
   rather than left to find two files.
5. **Why the speech models never downloaded.** Diagnose against the log, then
   fix. Same shape as item 2 — method now, patch when the evidence is in.

### Plan C2 — the settings features (additive, depends on C1)

Only reachable once the screen loads:

1. **Microphone picker.** Enumerate cpal input devices, show which is in use,
   let him change it, remember it. `notetaker-platform` must keep depending on
   no other notetaker crate — that property is what lets Linux cross-check the
   Windows and macOS code at all.
2. **Model status.** Downloaded or not, size, re-download, switch tier.
   `setup_status` already returns most of this.
3. **The Ollama model picker.** A dropdown over `OllamaStatus.models` writing
   to `settings.llmModel`, replacing the free-text field. Verifiable against
   his real server once C1 item 3 lands — which also closes the standing
   caveat that this module has only ever been tested against `httpmock`.
4. **Storage location.** Already built. Confirm it works once visible, and add
   the open-folder button, which is the only genuinely missing piece.

## Ground rules carried forward

- No modals, no blocking, no disabled buttons. Say the true thing once and get
  out of the way.
- Every user-visible sentence is written for someone who is not an engineer.
- Nothing deletes or rewrites audio. The unlink fix makes deletion *less*
  eager, never more.
- `notetaker-platform` depends on no other notetaker crate.
- The Windows-only paths cannot be tested on Linux. Anything that can be
  proven by a unit test on a fake gets one; anything that cannot is verified
  by running the built app on his machine, and is marked unverified until it
  has been.

## Out of scope

- Signing the installer. Costs money; his call.
- The library showing a live capture as `Recorded`. Cosmetic, already judged
  not worth an on-disk contract change.
- Playback. Already specced, planned, and building.
