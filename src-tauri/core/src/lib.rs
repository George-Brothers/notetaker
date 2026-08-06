//! Notetaker core: capture, storage, search index, processing queue, and the
//! diarize -> transcribe -> merge -> summarize pipeline.
//!
//! Platform-portable by construction: no Tauri, no UI, and no macOS-only API
//! outside a trait implementation. Where the platform is unavoidable — the
//! microphone, system audio, the machine's idle time — it sits behind a trait
//! with a working fake, so the logic above it is built and tested anywhere.

pub mod actions;
pub mod api;
pub mod capture;
pub mod dictation;
pub mod dispatch;
pub mod index;
pub mod logging;
pub mod models;
pub mod notes;
pub mod ollama;
pub mod paths;
pub mod pipeline;
pub mod power;
pub mod queue;
pub mod runtime;
pub mod scheduler;
pub mod storage;
pub mod templates;
pub mod transcript;
pub mod watch;

#[cfg(test)]
mod tests {
    #[test]
    fn harness_works() {
        assert_eq!(2 + 2, 4);
    }
}
