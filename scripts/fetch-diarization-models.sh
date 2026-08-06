#!/usr/bin/env bash
# Downloads the ONNX models needed for local speaker diarization
# (sherpa-onnx offline speaker diarization: pyannote segmentation +
# 3D-Speaker embedding extractor). Idempotent: skips files already present.
#
# Models come from k2-fsa/sherpa-onnx's own GitHub release assets:
#   https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-segmentation-models
#   https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models
# (the "recongition" typo is the project's real tag name, not ours)
set -euo pipefail

# sha256sum is GNU coreutils and is not on a stock Mac; shasum is, and is not
# on a stock Linux. Use whichever exists rather than requiring either.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}


ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS_DIR="$ROOT_DIR/models"
mkdir -p "$MODELS_DIR"

SEG_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
SEG_ARCHIVE="$MODELS_DIR/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
SEG_MODEL="$MODELS_DIR/sherpa-onnx-pyannote-segmentation-3-0/model.onnx"

EMB_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
EMB_MODEL="$MODELS_DIR/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"
EMB_SHA256="1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b"

if [ -f "$SEG_MODEL" ]; then
  echo "segmentation model already present: $SEG_MODEL"
else
  echo "downloading pyannote segmentation model..."
  curl -L -C - -o "$SEG_ARCHIVE" "$SEG_URL"
  tar xjf "$SEG_ARCHIVE" -C "$MODELS_DIR"
  rm -f "$SEG_ARCHIVE"
  if [ ! -f "$SEG_MODEL" ]; then
    echo "error: expected $SEG_MODEL after extracting archive" >&2
    exit 1
  fi
fi

if [ -f "$EMB_MODEL" ]; then
  echo "embedding model already present: $EMB_MODEL"
else
  echo "downloading 3D-Speaker embedding model..."
  curl -L -C - -o "$EMB_MODEL" "$EMB_URL"
fi

echo "verifying embedding model checksum..."
ACTUAL_SHA256="$(sha256_of "$EMB_MODEL")"
if [ "$ACTUAL_SHA256" != "$EMB_SHA256" ]; then
  echo "error: checksum mismatch for $EMB_MODEL" >&2
  echo "  expected: $EMB_SHA256" >&2
  echo "  actual:   $ACTUAL_SHA256" >&2
  exit 1
fi

echo "done."
echo "  segmentation: $SEG_MODEL"
echo "  embedding:    $EMB_MODEL"

# --- Diarization verification audio (committed to git, not downloaded) ------
# fixtures/diarization-check.wav is sherpa-onnx's own published multi-speaker
# test recording (real human voices), from:
#   https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-segmentation-models
# It is committed directly so `separates_speakers_on_real_multispeaker_audio`
# in diarize.rs runs without a network fetch. sherpa-onnx is Apache-2.0.
