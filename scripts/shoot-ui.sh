#!/usr/bin/env bash
# Screenshots the real UI (built frontend + notetaker-serve backend) in both
# themes. Usage: scripts/shoot-ui.sh <outdir>   → <outdir>/{light,dark}.png
set -euo pipefail
# Resolved to an absolute path before anything cds away: the screenshots are
# taken from src-tauri/, so a relative outdir would land somewhere else than
# the one mkdir created — and Chrome exits 0 when it cannot write the file, so
# the failure was silent and line 29 claimed success anyway.
OUT="${1:?usage: shoot-ui.sh <outdir>}"; mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd)"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHROME=$(ls -d ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1)

cd "$ROOT" && pnpm build
cd "$ROOT/src-tauri"
export PATH="$HOME/.cargo/bin:$PATH" LIBCLANG_PATH="$HOME/.local/lib/libclang"
cargo build -p notetaker-server --bin notetaker-serve
export LD_LIBRARY_PATH="$ROOT/src-tauri/target/debug"
PORT=14211
URL="http://127.0.0.1:$PORT"
LOG=$(mktemp)
./target/debug/notetaker-serve --port "$PORT" --ui-dir "$ROOT/dist" >"$LOG" 2>&1 &
SERVE_PID=$!
trap 'kill $SERVE_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  curl -sf "$URL" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -sf "$URL" >/dev/null 2>&1 || { echo "server never became reachable at $URL"; cat "$LOG"; exit 1; }
"$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --window-size=1280,800 --screenshot="$OUT/light.png" "${URL}?theme=light"
"$CHROME" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --window-size=1280,800 --screenshot="$OUT/dark.png" "${URL}?theme=dark"
echo "wrote $OUT/light.png and $OUT/dark.png"
