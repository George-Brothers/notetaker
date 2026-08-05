#!/usr/bin/env bash
# Renders Echo SVGs to PNGs and regenerates the full Tauri icon set.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/src-tauri/icons/source"
CHROME=$(ls -d ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1)

shot() { # $1 svg  $2 out.png  $3 size
  "$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
    --default-background-color=00000000 --window-size="$3,$3" \
    --screenshot="$2" "file://$1"
}

shot "$SRC/echo.svg" "$SRC/echo-1024.png" 1024
mkdir -p "$ROOT/src-tauri/icons/tray"
shot "$SRC/echo-tray-idle.svg"      "$ROOT/src-tauri/icons/tray/idle.png"      32
shot "$SRC/echo-tray-recording.svg" "$ROOT/src-tauri/icons/tray/recording.png" 32
shot "$SRC/echo-tray-paused.svg"    "$ROOT/src-tauri/icons/tray/paused.png"    32
cd "$ROOT" && pnpm tauri icon "$SRC/echo-1024.png"
echo "icons regenerated"
