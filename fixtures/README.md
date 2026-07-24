# `bilingual.wav` — bilingual two-speaker test fixture

## What it is

A synthetic audio clip used to gate transcription and diarization tests:

- **16 kHz, mono, 16-bit PCM WAV, ~34 seconds**
- **Two clearly different synthetic voices**, alternating, simulating a
  two-person meeting
- **Speaker A (English, male voice)** — includes the word "budget"
  (twice: "quarterly budget" and "budget spreadsheet")
- **Speaker B (Mandarin Chinese, female voice)** — real, grammatical
  Mandarin discussing the same meeting (预算 "budget", 招聘计划 "hiring
  plan")

The two speakers alternate three times each (six turns total), separated
by 0.8s of silence, so a diarizer has clean turn boundaries to work with
and a transcriber has unambiguous language switches.

Transcript:

| Speaker | Language | Text |
|---|---|---|
| A | en | Good morning everyone. Today we will review the quarterly budget and the hiring plan. |
| B | zh | 大家好。我们今天讨论预算和招聘计划。这个季度的收入增长了百分之十。 |
| A | en | That is great news. Let us schedule the follow up meeting for next Tuesday. |
| B | zh | 好的，没问题。下周二上午十点可以吗。 |
| A | en | Perfect. I will also send over the updated budget spreadsheet before then. |
| B | zh | 谢谢你。我也会把招聘计划的详细资料发给大家。会议结束后我们再确认最终预算。 |

## How it was generated

The task brief's original plan called for `espeak-ng` + `sox`. Neither is
installed on the build machine and there is no sudo available, so
generation was done instead with:

- **[piper-tts](https://github.com/OHF-Voice/piper1-gpl)** (`piper-tts`
  PyPI package, v1.6.0) — a local, offline neural TTS engine — installed
  into an isolated virtualenv via **`uv`** (`~/.local/bin/uv`), no root
  required.
- Two small ONNX voice models, downloaded once via piper's own voice
  downloader (`python -m piper.download_voices`) from the public Piper
  voice registry:
  - **`en_US-lessac-medium`** — English (US), male-sounding voice, for
    Speaker A
  - **`zh_CN-huayan-medium`** — Mandarin Chinese, female-sounding voice,
    for Speaker B
- **`ffmpeg`** (already on `PATH`) to resample each synthesized clip to
  16 kHz mono, generate 0.8s silence gaps, and concatenate everything
  into the final `bilingual.wav`.

No network TTS API was used and no third-party recorded audio was
downloaded — synthesis itself is fully local/offline once the voice
model files are cached; only the one-time model download touches the
network.

Regenerate with:

```bash
./fixtures/make_fixture.sh
```

This is idempotent in kind (same two voices, same script, same 16 kHz
mono format) though exact sample bytes may differ slightly across piper
releases.

## Verification (last generated)

```
$ ffprobe fixtures/bilingual.wav
Duration: 00:00:34.21, ... 16000 Hz, 1 channels, s16

$ ffmpeg -i fixtures/bilingual.wav -af volumedetect -f null -
mean_volume: -16.8 dB
max_volume: 0.0 dB
```

Real signal (not silence), correct format, duration within the required
25–60s window.

## Contingency note (from the task brief, verbatim in substance)

If the diarizer in Task 7 cannot separate these two synthetic voices,
replace this file with a short CC-licensed real two-speaker EN/ZH clip
and record its provenance here (source, license, URL). The test
contract does not change: the fixture must live at `fixtures/bilingual.wav`
and must yield **≥2 speakers** and **both languages (English + Mandarin)**
when run through the transcription/diarization pipeline. As of this
writing the fixture is still the synthetic piper-tts recording described
above — no replacement has been needed.
