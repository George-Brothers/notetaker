# Active product

- The repository root is the active React + Tauri 2 Notetaker. The branch was
  `main` and clean when verified on 2026-08-12.
- `src/` is the UI; the Rust workspace is `src-tauri/` (`core`, `platform`,
  `server`, and the Tauri shell). `docs/superpowers/` is historical design and
  review material, not automatic workflow authority.

# Golden runtime path

- Install: `pnpm install`.
- Native development: `pnpm run tauri dev`. `pnpm dev` serves the frontend only and
  does not prove Tauri IPC, permissions, capture, or local model behavior.
- The installed acceptance surface is `/Applications/Notetaker.app` (bundle
  `com.georgebrothers.notetaker`, version 0.1.11 when inspected).
- There is no login. User recordings live under `~/Notetaker`; disposable app
  state/settings/index live under `~/Library/Application Support/Notetaker`.
  Do not inspect or alter real recordings merely to test a change.
- Development/server diagnostics are terminal output. The served Rust binary
  installs a stdout/stderr logger; the Tauri shell has no persistent log sink
  configured, so do not claim a file log exists.

# Verification

- `pnpm test` runs frontend Vitest only. `pnpm build` runs TypeScript plus Vite;
  neither proves the native app.
- From `src-tauri/`, run
  `cargo test -p notetaker-core -p notetaker-platform -p notetaker-server` for
  portable Rust logic and `cargo clippy --all-targets` when appropriate.
- `scripts/check-platforms.sh` is compile evidence for platform code, not native
  runtime proof. Native acceptance requires launching the real app, permissions,
  actual UI/runtime observation, and appropriate safe fixture data.
- macOS system audio remains deliberately unimplemented in
  `src-tauri/platform/src/macos/speaker.rs`; meeting mode must refuse rather
  than silently capture only the microphone.

# Stable architecture constraints

- `~/Notetaker` is a cross-platform public data-layout contract. The SQLite
  index must remain rebuildable; recordings and user-authored `notes.md` are
  never disposable.
- Desktop and served UI share `core::dispatch`; preserve loopback-by-default and
  token-required LAN serving.

# Dangerous boundaries

- Never delete, rewrite, or move live recordings, user notes, models, settings,
  or installed apps without explicit authority. Signing, packaging, installer,
  release, push, and deployment work is `SHIP`.

# Current state

- Root, branch, package/Cargo commands, storage paths, installed app, logging,
  and the macOS capture boundary were inspected 2026-08-12.
