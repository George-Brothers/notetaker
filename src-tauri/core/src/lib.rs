//! Notetaker core: storage, search index, processing queue, and the
//! diarize -> transcribe -> merge -> summarize pipeline.
//!
//! Platform-portable by construction: no Tauri, no UI, no macOS-only APIs.

pub mod api;
pub mod index;
pub mod models;
pub mod pipeline;
pub mod queue;
pub mod scheduler;
pub mod storage;

#[cfg(test)]
mod tests {
    #[test]
    fn harness_works() {
        assert_eq!(2 + 2, 4);
    }
}
