# ASR bake-off result — which engine transcribes English + Chinese

**Date:** 2026-07-23 · **Fixture:** `fixtures/bilingual.wav` (34 s, EN male +
ZH female, alternating) scored against `fixtures/bilingual.reference.txt`.
**Metric:** character error rate (lower is better), split by script.

## Measured (this Linux box, small/CPU-tier models)

| Engine | Model | Time | CER overall | CER English | CER Chinese |
|---|---|---:|---:|---:|---:|
| Whisper | `ggml-tiny` (multilingual) | 3.7 s | 15.6% | **0.0%** | **53.2%** |
| SenseVoice | `sense-voice-zh-en-...-2024-07-17 int8` | 3.5 s | **4.1%** | 3.7% | **8.9%** |

Sample outputs (Chinese portion):
- **Whisper tiny:** `照拼計畫` (should be `招聘计划`), `基督的受入` (should be
  `季度的收入`) — and it emitted **traditional** characters. Frequent
  wrong-homophone errors; unusable for Chinese at this tier.
- **SenseVoice:** `招聘计划`, `预算`, `会议结束后我们再确认最终预算` — correct
  simplified characters, minor dropped particles only.

## Decision

**SenseVoice is the default speech engine for the small / CPU tiers.** On the
one axis that matters most for this user — mixed English *and* Chinese —
Whisper-tiny is not close (53% vs 9% Chinese CER), so the plan's "if results
are close, Whisper wins" tie-break does not apply. SenseVoice is also as fast
and one model covers both languages.

Whisper stays in the codebase (the `WhisperTranscriber` and the whole
`Transcriber` trait are unchanged) as the alternate/fallback engine.

## Must re-run on the Mac before final lock

This compared the **small** tier only. The plan's top tier is Whisper
`large-v3-turbo`, which is dramatically better at Chinese than `tiny` and may
close the gap on Apple-Silicon-Big machines. Re-run this exact bake-off on the
Mac with `large-v3-turbo` vs SenseVoice before hard-coding the tier→engine
mapping. Command:

```
cargo run -p notetaker-core --bin bakeoff -- \
    fixtures/bilingual.wav fixtures/bilingual.reference.txt \
    --whisper models/ggml-large-v3-turbo.bin \
    --sensevoice models/sherpa-onnx-sense-voice-.../model.int8.onnx
```

The fixture is synthetic TTS; a real bilingual human recording would make the
final call more trustworthy still (see the diarization note in
`fixtures/README.md` for the same caveat).

---

## Re-run on real human audio, 2026-07-30 — and the fixture caveat closed

The note above asked for "a real bilingual human recording". Mr. Brothers
pointed at his own Meetily archive, so this was re-run on two 90-second windows
of real meetings — spontaneous speech, cross-talk, background noise, and
code-switching **inside single sentences**. Whisper `small-q5_1` (this machine's
tier) against the router.

| | Chinese-heavy window | Second window |
|---|---|---|
| Whisper alone | 200.5 s | 229.6 s |
| Routed | 75.6 s | 131.2 s |
| Speed-up | **2.65×** | **1.75×** |
| CJK characters recovered | 35 → 46 | 21 → 48 |

### The safety result, which mattered more than the speed

Of 23 timestamps present in both runs of the second window, **14 came out byte
for byte identical — and every one of them is an English sentence.** Routing
detected them as English, sent them to Whisper, and got exactly what Whisper
alone produced. That is the guarantee worth having: *turning routing on cannot
degrade English*, because English is still transcribed by the English model.

### Where the nine differences fall

Seven are the router winning, all on Chinese:

| Whisper alone | Routed |
|---|---|
| `Usually, well sometimes, you do shen ma like shen ma.` | `Uually, well sometimes you do什么 like什么。` |
| `Also, not flat. Second, shem.` | `Also, not flat. second什么什么？` |
| `DR Shama.` | `点ear什么？` |
| `就在一下什么` | `就是来一下什么。` |

Whisper writes Mandarin as *romanized English words* — `shen ma` for `什么`,
and an invented proper noun `DR Shama`. The text is not merely less accurate,
it is unsearchable: nobody looking for `什么` will ever find `shen ma`.

Two are the router losing, both on **non-speech**: where Whisper emits
`[MUSIC PLAYING]` and `[BLANK_AUDIO]`, SenseVoice hallucinates a short Japanese
interjection (`あ。`, `い？`). Neither output is speech; Whisper's at least
labels itself as noise. **Filtering non-speech from both engines is a separate,
worthwhile fix** and is listed under "Next" in `docs/MAP.md`.

### A correction, because the premise was wrong

This second window was chosen as a **pure-English control** — Meetily's own
transcript of that meeting contains 138k characters and *zero* CJK. That fact
turned out to mean the opposite of what it looked like: the meeting is also a
Chinese lesson, and Meetily's zero-CJK transcript is evidence that **its** model
failed on the Chinese entirely, romanizing it. There was no English control in
this archive to find; both meetings are bilingual.

The first attempt at this control was also thrown away: the 90 seconds picked
were nearly silent, and both engines hallucinated Mandarin onto background
noise. The window used above is the densest 90 seconds of actual speech in the
recording, chosen by measuring, not by guessing.

### Still to do on the Mac

Unchanged: this is the **small** Whisper tier. `large-v3-turbo` is much better
at Chinese and may narrow the quality gap on Apple-Silicon-Big — though not the
speed gap, which runs the other way, since large-v3-turbo is slower still.
