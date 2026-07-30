#!/usr/bin/env bash
# Type-checks the platform layer against macOS and Windows from any machine.
#
# This is the fast loop that keeps the per-OS capture code from being written
# blind. `cargo check` does not link, and `notetaker-platform` deliberately
# depends on no C or C++ — so both real targets type-check here with no
# cross-compiler and no SDK, in seconds.
#
# What it proves:   the platform code compiles for the target, against the
#                   actual OS bindings.
# What it does not: that it links, that the OS returns what its documentation
#                   promises, or that any test passes. Only CI (windows-latest,
#                   macos-14) and a human at the machine can say that.
#
# Usage: scripts/check-platforms.sh [--install-targets]
set -euo pipefail

cd "$(dirname "$0")/../src-tauri"

TARGETS=(x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin)

if [[ "${1:-}" == "--install-targets" ]]; then
  for t in "${TARGETS[@]}"; do
    echo "==> rustup target add $t"
    rustup target add "$t"
  done
fi

missing=()
installed="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
  grep -qx "$t" <<<"$installed" || missing+=("$t")
done
if ((${#missing[@]})); then
  echo "Missing rust targets: ${missing[*]}" >&2
  echo "Run: scripts/check-platforms.sh --install-targets" >&2
  exit 1
fi

failed=()
for t in "${TARGETS[@]}"; do
  echo
  echo "=============================================================="
  echo "  cargo check -p notetaker-platform --target $t"
  echo "=============================================================="
  if cargo check -p notetaker-platform --all-targets --target "$t"; then
    echo "  OK: $t"
  else
    echo "  FAILED: $t"
    failed+=("$t")
  fi
done

echo
echo "=============================================================="
echo "  cargo test -p notetaker-platform   (pure layer, this machine)"
echo "=============================================================="
if ! cargo test -p notetaker-platform; then
  failed+=("host tests")
fi

echo
if ((${#failed[@]})); then
  echo "FAILED: ${failed[*]}" >&2
  exit 1
fi
echo "All platform targets type-check and the pure layer passes."
echo
echo "Reminder: compile-verified is not run-verified. The capture path is"
echo "first actually exercised on real hardware."
