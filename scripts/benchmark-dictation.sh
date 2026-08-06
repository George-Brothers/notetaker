#!/usr/bin/env bash
# Measures the warm release-to-text path for system-wide dictation.
#
# The speech model and Silero VAD are loaded before the timed section, matching
# the production ModelCache lease acquired on key-press. The benchmark uses a
# short real 16 kHz fixture and runs the exact VAD, fresh WhisperState, prompt,
# and cleanup code used by dictation. No network endpoint is accepted: Ollama
# is fixed to the loopback address inside the Rust harness.
#
# Usage:
#   scripts/benchmark-dictation.sh [cleanup-model]
#
# The Whisper/VAD paths are overrideable for a CI or a downloaded local model:
#   WHISPER_MODEL=/path/ggml-tiny.bin VAD_MODEL=/path/silero_vad.onnx \
#     scripts/benchmark-dictation.sh llama3.2:3b
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHISPER_MODEL="${WHISPER_MODEL:-$ROOT/models/ggml-tiny.bin}"
VAD_MODEL="${VAD_MODEL:-/tmp/notetaker-silero-vad.onnx}"
CLEANUP_MODEL="${1:-llama3.2:3b}"

for required in "$ROOT/fixtures/bilingual.wav" "$WHISPER_MODEL" "$VAD_MODEL"; do
  if [[ ! -f "$required" ]]; then
    echo "missing benchmark input: $required" >&2
    echo "download the local speech/VAD artifacts first; nothing is fetched by this harness" >&2
    exit 2
  fi
done

if ! curl --max-time 3 -fsS http://127.0.0.1:11434/api/tags >/dev/null; then
  echo "local Ollama is not reachable at 127.0.0.1:11434" >&2
  exit 2
fi

cd "$ROOT/src-tauri"
CARGO_BIN="$(command -v cargo || true)"
if [[ -z "$CARGO_BIN" ]] && command -v rustup >/dev/null 2>&1; then
  CARGO_BIN="$(rustup which cargo 2>/dev/null || true)"
fi
if [[ -z "$CARGO_BIN" || ! -x "$CARGO_BIN" ]]; then
  echo "cargo is not available; add Rust's cargo binary to PATH" >&2
  exit 2
fi
RUSTC_BIN=""
if command -v rustup >/dev/null 2>&1; then
  RUSTC_BIN="$(rustup which rustc 2>/dev/null || true)"
fi
TOOLCHAIN_BIN="$(dirname "$CARGO_BIN")"
if [[ -n "$RUSTC_BIN" ]]; then
  TOOLCHAIN_BIN="$(dirname "$RUSTC_BIN"):$TOOLCHAIN_BIN"
fi
export PATH="$TOOLCHAIN_BIN:$PATH"
exec "$CARGO_BIN" run -p notetaker-core --example dictation-latency -- \
  "$ROOT/fixtures/bilingual.wav" "$WHISPER_MODEL" "$VAD_MODEL" "$CLEANUP_MODEL"
