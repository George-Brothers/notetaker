#!/usr/bin/env bash
# Downloads the multilingual "tiny" whisper.cpp ggml model used by the
# WhisperTranscriber tests, into models/ (gitignored) at the repo root.
#
# Usage: scripts/fetch-whisper-model.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS_DIR="$REPO_ROOT/models"
MODEL_PATH="$MODELS_DIR/ggml-tiny.bin"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
# Official sha256 of ggml-tiny.bin, as published by the Hugging Face
# git-lfs metadata for ggerganov/whisper.cpp (verified 2026-07-23).
EXPECTED_SHA256="be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"

mkdir -p "$MODELS_DIR"

echo "Fetching $MODEL_URL -> $MODEL_PATH"
curl -L -C - -o "$MODEL_PATH" "$MODEL_URL"

echo "Verifying sha256..."
ACTUAL_SHA256="$(sha256sum "$MODEL_PATH" | awk '{print $1}')"

if [ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]; then
    echo "ERROR: sha256 mismatch for $MODEL_PATH" >&2
    echo "  expected: $EXPECTED_SHA256" >&2
    echo "  actual:   $ACTUAL_SHA256" >&2
    exit 1
fi

echo "OK: $MODEL_PATH verified ($ACTUAL_SHA256)"
