# Listen — playback on every recording

2026-07-30. Mr. Brothers, after using the app on his own PC:

> playback on every recording, processed or not — raw audio listenable the
> second recording stops

and, on placement:

> i want it to be an option icon in the row with all the other ones like
> process now ask etc and then if you click it then it becomes the thin bar
> under the title

and, on the controls:

> make sure it has a lot of well made controls like plus 10 seconds and minus
> 10 seconds and then also a way to minimize it back into the row

## Why this is not a new feature

The player already exists and already works — play, restart, scrub, track
picker, click-a-line-to-seek, and line highlighting that follows playback. It
was verified in a browser against the served UI.

It is simply unreachable. `TranscriptPanel` returns early when a recording has
no segments (`TranscriptPanel.tsx`, the `detail.segments.length === 0` guard),
and the player is built *below* that return. An unprocessed recording therefore
gets one sentence — "The transcript appears here once this recording has been
processed" — and no player, even though the audio is on disk and the
`audio_path` command serves it happily.

So the work is: move the player somewhere a recording without a transcript can
reach it, and make it worth reaching.

## The second bug, found while reading the code

`audio_tracks` in `core/src/api.rs` lists a track when the file merely
**exists**. Mr. Brothers' real recordings revealed the true shape of the bug:

- `2026-07-30 17.38 Meeting/` — `audio-system.flac` 4,469 bytes
- `2026-07-30 17.40 Meeting/` — `audio-system.wav` 44 bytes
- `2026-07-30 17.44 In person/` — (no system track at all)

The WAV is header-only: WASAPI loopback wrote the chunk headers but no samples
because nothing was playing through his speakers. That 44-byte file passes
`.exists()`, so it is offered as a track. Worse, `TranscriptPanel` *prefers*
`system` when it is present, on the reasoning that the other people in a
meeting are on that track.

Net effect on his machine: the player defaulted to the header-only WAV on the
one recording that had both tracks, and played silence. (The FLAC alongside it
is quiet but real — quiet is not the same as absent.) Fixing placement without
fixing this ships a player that appears broken.

**Fix:** `audio_tracks` now distinguishes between files with content and files
without it — for FLACs by checking `len() > 0` (an encoded file has audio), for
WAVs by opening with `hound` and reading the frame count directly. A WAV with
no frames is not a track, even if it is readable and 44 bytes on disk.

## The design

### The control lives in the toolbar

A `Listen` toggle joins the note's toolbar row beside `Ask` and `Process now`
(`NoteView.tsx`, the row right of the `TabList`). Headphones icon, same
`aria-pressed` treatment `Ask` already uses, so the pressed state is consistent
with what is there.

Off by default. Resets when a different recording is opened.

### Pressing it opens a thin bar under the title

One line, below the title/meta block and above the tabs:

```
⏮  ⏪10  ▶  ⏩10   ────────●──────────   2:14 / 12:34   1×   mic ▾   ⌄
```

Left to right:

| Control | Behaviour |
|---|---|
| Restart | Seeks to 0. Does not change play state. |
| −10s | `seek(currentTime - 10)`, clamped at 0. |
| Play / Pause | Primary control, visually the largest. |
| +10s | `seek(currentTime + 10)`, clamped at duration. |
| Scrub | Range input over the full duration; drag or click to seek. |
| Elapsed / total | `2:14 / 12:34`. |
| Speed | Popover: 0.75×, 1×, 1.25×, 1.5×, 1.75×, 2×. Sets `playbackRate`. |
| Track | Popover, **only when more than one track has bytes**. Otherwise a plain label naming the track you are hearing. |
| Collapse | Minimises the bar back into the toolbar row. Same state the `Listen` toggle owns. |

The Speed and Track popovers use the `Popover` primitive already used by the
template picker, so a third interaction idiom is not introduced.

### One player, never two

`useAudio` moves out of `TranscriptPanel` and up into `NoteView`, which passes
it down. `TranscriptPanel` stops owning an `<audio>` element and stops rendering
its own transport strip — it keeps only what is genuinely transcript work:
click-a-line-to-seek and highlight-the-line-that-is-playing, both driven by the
shared state.

This is the load-bearing part of the design. Two independent `<audio>` elements
in one view means two things can play at once, and the transcript can highlight
a line that a different player is nowhere near.

Opening the Transcript tab force-opens the bar. Seeking to a line with no player
on screen is meaningless, and the highlight has to be explained by something
visible. It can still be collapsed from there — force-open is a default, not a
lock.

### A recording that is still recording

The library shows a live capture as `Recorded` (a known cosmetic gap), so its
note *can* be opened mid-capture and `Listen` *will* be pressed. The file on
disk is a WAV still being appended to: its duration is wrong and it is finalised
to FLAC the moment capture stops.

The bar therefore refuses, in the same plain-English register `runtime.rs`
already uses for renaming a live recording:

> This recording is still going. It'll be listenable as soon as you stop.

`useCapture` already polls `capture_status`, which carries `recordingId`. That
id is threaded down to `NoteView`; when it matches the open recording, the bar
shows the sentence instead of a transport. This is a UI courtesy, not a
guarantee — the guarantees live in `runtime.rs`, and playback only ever reads.

### Collapsing does not stop playback

"Minimise", not "close". Audio keeps playing so it can run while notes are being
typed. To keep that from becoming a mystery sound, the `Listen` toggle in the
toolbar carries the pressed/accent state while audio is playing, so the source
is always one glance away.

### Keyboard

Only while focus is inside the bar, and never when focus is in an input,
textarea, or contenteditable — the notepad shares this view, and stealing Space
from someone typing notes is unacceptable.

| Key | Action |
|---|---|
| Space | Play / pause |
| ← / → | −10s / +10s |

### When there is no audio at all

No disabled buttons, per his standing rule — *"the app should be like okay fine
but just so u know it wont work."* `Listen` still presses. The bar opens and
says, in place of the transport:

> No audio was saved for this recording.

Said once, plainly, and out of the way.

### What persists

| | |
|---|---|
| Bar open/closed | Resets per recording. Closed by default. |
| Playback position | Resets per recording. |
| Speed | Persists in `localStorage`, in the guarded try/catch style `useTheme.ts` and `App.tsx` already use. Someone who likes 1.5× likes it on the next recording too. |
| Track | Resets per recording, to the first track with bytes. |

## Testing

Behaviour, not appearance — there is still no display in this environment.

- An **unprocessed** recording (no segments, no transcript) renders a working
  player. This is the regression that started the whole item.
- Exactly **one** `<audio>` element is present regardless of which tab is open
  and whether the bar was opened from the toolbar or by the Transcript tab.
- ±10s clamp at both ends rather than seeking past them.
- Collapsing the bar does not pause.
- Switching to the Transcript tab opens the bar.
- The keyboard handler ignores events originating in a text field.
- The recording that is currently capturing shows the still-going sentence
  rather than a transport, and a different recording opened during that same
  capture does not.
- **Core:** `audio_tracks` omits a WAV with no audio frames (header-only), and
  still lists a track that has audio. Directly pins the bug from his real
  recordings.
- The no-audio wording, since on that path the wording is the entire feature.

## Out of scope

- **Play buttons on library rows.** Offered and declined — the sidebar stays a
  navigation list, and app-wide player state is not worth it for a hover button.
- **Volume.** The OS owns it.
- **Waveform.** Nothing asked for it and it costs real work.
- The stray `audio-mic.wav` sitting beside the `.flac` on two of three
  recordings. Related to the header-only WAV finding but a separate failure —
  FLAC verification not confirming while `meta.error` stays `null` — and it is
  its own item on the work list.
