# Notetaker

Notetaker is a local-first desktop app for recording meetings and classes,
transcribing them on your computer, and turning them into searchable notes.
Audio, transcripts, notes, and downloaded speech models stay in your local
Notetaker library.

## Install

Download the latest Windows installer from the project's
[Releases page](https://github.com/George-Brothers/notetaker-public/releases).
The app checks for signed updates automatically while it is idle; it never
restarts a recording to update itself.

## What it does

- Record microphone audio and supported meeting-app system audio.
- Play recordings, whether or not they have been processed yet.
- Transcribe locally with downloadable speech models.
- Create summaries, action items, and task suggestions with Ollama or another
  compatible local model endpoint.
- Archive, restore, or permanently delete recordings with confirmation.

## Development

Requirements: Node 20+, pnpm 9+, Rust stable, and platform build tools.

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm build
pnpm tauri dev
```

The Rust core can be tested from `src-tauri` with:

```sh
cargo test -p notetaker-core --lib
```

## Privacy and releases

The app is designed to keep recordings and processing local. The public
release feed contains only signed installers and updater metadata. Third-party
attribution is in [NOTICE](NOTICE).
