//! Processing pipeline stages. Each stage is an independent module; `run`
//! orchestrates them.

pub mod ask;
pub mod audio;
pub mod diarize;
pub mod llm;
pub mod merge;
pub mod run;
pub mod suggest;
pub mod summarize;
pub mod transcribe;

/// One labeled span of speech in a finished transcript.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Utterance {
    pub start_s: f32,
    pub end_s: f32,
    pub speaker: String,
    pub text: String,
}
