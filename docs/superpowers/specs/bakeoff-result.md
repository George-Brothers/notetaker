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
