#!/usr/bin/env bash
# Measure the Phase 1 idle-RAM acceptance gate on macOS.
#
# Usage:
#   scripts/accept-idle-ram.sh <notetaker-pid> [idle-seconds] [tick-seconds]
#
# Build/run the app in a debug configuration and set modelIdleUnload to "15s"
# before starting this script. The batch itself is manual because the script
# cannot safely invent a recording or an Ollama/model setup. The three samples
# are taken with Apple's footprint tool, not RSS.

set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
  echo "usage: $0 <notetaker-pid> [idle-seconds=15] [tick-seconds=30]" >&2
  exit 2
fi

PID="$1"
IDLE_SECONDS="${2:-15}"
TICK_SECONDS="${3:-30}"

if ! [[ "$PID" =~ ^[0-9]+$ && "$IDLE_SECONDS" =~ ^[0-9]+$ && "$TICK_SECONDS" =~ ^[0-9]+$ ]]; then
  echo "pid and wait intervals must be non-negative integers" >&2
  exit 2
fi

if ! command -v footprint >/dev/null 2>&1; then
  echo "footprint is required; this acceptance gate must run on macOS" >&2
  exit 2
fi

REPORT="$(mktemp -t notetaker-idle-ram).txt"
echo "Raw footprint report: $REPORT"

extract_bytes() {
  awk '
    BEGIN { IGNORECASE = 1 }
    /phys_footprint/ {
      for (i = 1; i <= NF; i++) {
        token = $i
        gsub(/[^0-9]/, "", token)
        if (token != "") {
          print token
          exit
        }
      }
    }
  '
}

measure() {
  local label="$1"
  local raw
  local bytes

  raw="$(footprint -p "$PID" -f bytes)"
  {
    echo "[$label]"
    echo "$raw"
  } >>"$REPORT"

  bytes="$(printf '%s\n' "$raw" | extract_bytes)"
  if [[ -z "$bytes" ]]; then
    echo "could not find phys_footprint in the $label sample; raw output is in $REPORT" >&2
    exit 1
  fi
  echo "$bytes"
}

echo "Sample 1/3: leave Notetaker before its speech models are loaded."
read -r -p "Press Return to capture the baseline: " _
BASELINE="$(measure baseline)"
echo "baseline: $BASELINE bytes ($((BASELINE / 1024 / 1024)) MiB)"

echo "Process exactly one recording batch now. Keep the app open after it reaches Ready."
read -r -p "Press Return after the processed batch has landed: " _
AFTER_BATCH="$(measure after-batch)"
echo "after batch: $AFTER_BATCH bytes ($((AFTER_BATCH / 1024 / 1024)) MiB)"

WAIT_SECONDS=$((IDLE_SECONDS + TICK_SECONDS + 1))
echo "Waiting ${WAIT_SECONDS}s: idle window (${IDLE_SECONDS}s) plus one scheduler tick (${TICK_SECONDS}s)."
sleep "$WAIT_SECONDS"
AFTER_IDLE="$(measure after-idle-tick)"
echo "after idle tick: $AFTER_IDLE bytes ($((AFTER_IDLE / 1024 / 1024)) MiB)"

DELTA=$((AFTER_IDLE - BASELINE))
if (( DELTA < 0 )); then
  DELTA=$((-DELTA))
fi
LIMIT=$((50 * 1024 * 1024))

echo "absolute baseline-to-idle delta: $DELTA bytes ($((DELTA / 1024 / 1024)) MiB)"
if (( DELTA <= LIMIT )); then
  echo "PASS: the third footprint is within 50 MiB of the first."
  exit 0
fi

echo "FAIL: the third footprint is not within 50 MiB of the first; inspect $REPORT" >&2
exit 1
