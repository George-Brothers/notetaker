#!/usr/bin/env bash
# Renders Echo SVGs to PNGs and regenerates the full Tauri icon set.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src-tauri/icons/source"
CHROME=""
for candidate in ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome; do
  if [[ -x "$candidate" ]]; then
    CHROME="$candidate"
    break
  fi
done
SWIFT_CACHE="${TMPDIR:-/tmp}/notetaker-swift-module-cache"

shot() { # $1 svg  $2 out.png  $3 size
  if [[ -n "$CHROME" ]]; then
    "$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
      --default-background-color=00000000 --window-size="$3,$3" \
      --screenshot="$2" "file://$1"
  else
    mkdir -p "$SWIFT_CACHE"
    swift -module-cache-path "$SWIFT_CACHE" "$ROOT/scripts/render-svg.swift" "$1" "$2" "$3"
  fi
}

shot "$SRC/echo.svg" "$SRC/echo-1024.png" 1024
mkdir -p "$ROOT/src-tauri/icons/tray"
shot "$SRC/echo-tray-idle.svg"      "$ROOT/src-tauri/icons/tray/idle.png"      32
shot "$SRC/echo-tray-recording.svg" "$ROOT/src-tauri/icons/tray/recording.png" 32
shot "$SRC/echo-tray-paused.svg"    "$ROOT/src-tauri/icons/tray/paused.png"    32

# macOS status items are template images: black mask plus alpha, no colored
# background. Keep both representations because AppKit uses the @2x image on
# Retina menu bars while the @1x files make the asset contract explicit.
MACOS_TRAY="$ROOT/src-tauri/icons/tray/macos"
mkdir -p "$MACOS_TRAY"
for state in idle recording paused; do
  shot "$SRC/echo-tray-macos-$state.svg" "$MACOS_TRAY/$state.png" 18
  shot "$SRC/echo-tray-macos-$state.svg" "$MACOS_TRAY/$state@2x.png" 36
done
cd "$ROOT" && pnpm tauri icon "$SRC/echo-1024.png"
echo "icons regenerated"
