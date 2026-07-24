#!/usr/bin/env bash
# Regenerates fixtures/bilingual.wav.
#
# The task brief's original plan (espeak-ng + sox) could not be used: neither
# tool is installed on this box and there is no sudo. This script instead
# uses:
#   - uv (https://github.com/astral-sh/uv) to create an isolated Python venv
#     and install piper-tts (https://github.com/OHF-Voice/piper1-gpl), a
#     local/offline neural TTS engine, with no root required.
#   - piper's own voice downloader to fetch two small ONNX voice models
#     (one en_US, one zh_CN) from the public Piper voice registry on Hugging
#     Face. No network TTS API is used — synthesis itself runs fully local
#     and offline once the voice files are cached.
#   - ffmpeg (already on PATH) to resample, add silence gaps, and concatenate
#     the four synthesized clips into the final fixture.
#
# Re-running this script regenerates a fixture of the same kind: same two
# voices, same script text, same 16 kHz mono format. Exact sample content can
# differ slightly between piper releases, but language, speaker identity, and
# structure are stable.
set -euo pipefail
cd "$(dirname "$0")"

UV="${UV:-$HOME/.local/bin/uv}"
if ! command -v "$UV" >/dev/null 2>&1; then
  echo "error: uv not found (looked for $UV). Install uv or set UV=/path/to/uv" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

VENV="$WORK/venv"
VOICES="$WORK/voices"
mkdir -p "$VOICES"

echo "== creating venv and installing piper-tts =="
"$UV" venv "$VENV" >/dev/null
"$UV" pip install --python "$VENV/bin/python" piper-tts >/dev/null
PY="$VENV/bin/python"

EN_VOICE=en_US-lessac-medium
ZH_VOICE=zh_CN-huayan-medium

echo "== downloading voices (en=$EN_VOICE, zh=$ZH_VOICE) =="
"$PY" -m piper.download_voices "$EN_VOICE" --download-dir "$VOICES"
"$PY" -m piper.download_voices "$ZH_VOICE" --download-dir "$VOICES"

synth() {
  local text="$1" model="$2" out="$3"
  printf '%s' "$text" | "$PY" -m piper -m "$VOICES/$model.onnx" -f "$out"
}

echo "== synthesizing speech =="
synth "Good morning everyone. Today we will review the quarterly budget and the hiring plan." \
  "$EN_VOICE" "$WORK/a1_raw.wav"
synth "大家好。我们今天讨论预算和招聘计划。这个季度的收入增长了百分之十。" \
  "$ZH_VOICE" "$WORK/b1_raw.wav"
synth "That is great news. Let us schedule the follow up meeting for next Tuesday." \
  "$EN_VOICE" "$WORK/a2_raw.wav"
synth "好的，没问题。下周二上午十点可以吗。" \
  "$ZH_VOICE" "$WORK/b2_raw.wav"
synth "Perfect. I will also send over the updated budget spreadsheet before then." \
  "$EN_VOICE" "$WORK/a3_raw.wav"
synth "谢谢你。我也会把招聘计划的详细资料发给大家。会议结束后我们再确认最终预算。" \
  "$ZH_VOICE" "$WORK/b3_raw.wav"

echo "== resampling to 16 kHz mono =="
for f in a1 b1 a2 b2 a3 b3; do
  ffmpeg -y -loglevel error -i "$WORK/${f}_raw.wav" -ar 16000 -ac 1 "$WORK/${f}.wav"
done

echo "== building 0.8s silence gap =="
ffmpeg -y -loglevel error -f lavfi -i anullsrc=r=16000:cl=mono -t 0.8 -c:a pcm_s16le "$WORK/sil.wav"

echo "== concatenating =="
CONCAT_LIST="$WORK/concat.txt"
{
  echo "file '$WORK/a1.wav'"
  echo "file '$WORK/sil.wav'"
  echo "file '$WORK/b1.wav'"
  echo "file '$WORK/sil.wav'"
  echo "file '$WORK/a2.wav'"
  echo "file '$WORK/sil.wav'"
  echo "file '$WORK/b2.wav'"
  echo "file '$WORK/sil.wav'"
  echo "file '$WORK/a3.wav'"
  echo "file '$WORK/sil.wav'"
  echo "file '$WORK/b3.wav'"
} > "$CONCAT_LIST"

ffmpeg -y -loglevel error -f concat -safe 0 -i "$CONCAT_LIST" -c copy bilingual.wav

echo "wrote $(pwd)/bilingual.wav"
ffprobe -hide_banner "bilingual.wav"
