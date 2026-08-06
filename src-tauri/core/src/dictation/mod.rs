//! System-wide dictation's portable core: state shapes, transcript cleanup,
//! local history, and the model-facing processing step. Runtime owns the
//! microphone thread and platform paste boundary; this module keeps those
//! pieces independently testable.

pub mod cleanup;
pub mod history;
pub mod vad;

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::pipeline::llm::LlmClient;
use crate::pipeline::transcribe::Transcriber;

pub use history::{DictationEntry, DictationHistory};
pub use vad::{SileroGate, VadSmoother};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationState {
    Idle,
    Recording,
    Transcribing,
    Pasting,
    Error,
}

impl DictationState {
    pub fn active(self) -> bool {
        !matches!(self, Self::Idle | Self::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationStatus {
    pub state: DictationState,
    pub elapsed_s: f64,
    pub level: f32,
    pub text: String,
    pub message: Option<String>,
}

impl Default for DictationStatus {
    fn default() -> Self {
        Self {
            state: DictationState::Idle,
            elapsed_s: 0.0,
            level: 0.0,
            text: String::new(),
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteResult {
    pub inserted: bool,
    pub clipboard_restored: bool,
    pub message: String,
}

impl PasteResult {
    pub fn inserted(message: impl Into<String>) -> Self {
        Self {
            inserted: true,
            clipboard_restored: true,
            message: message.into(),
        }
    }

    pub fn copied(message: impl Into<String>) -> Self {
        Self {
            inserted: false,
            clipboard_restored: false,
            message: message.into(),
        }
    }
}

/// Settings captured at key-press time. A run must not change behavior in the
/// middle because the user saves Settings while speaking.
#[derive(Debug, Clone)]
pub struct DictationConfig {
    pub cleanup_enabled: bool,
    pub cleanup_model: String,
    pub llm_base_url: String,
    pub dictionary: Vec<String>,
    pub replacements: BTreeMap<String, String>,
    pub keep_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupOutcome {
    pub text: String,
    pub warning: Option<String>,
    pub used_llm: bool,
}

/// Makes a bounded Whisper initial prompt from the user's dictionary and
/// replacement keys. It is a hint, never a transcript instruction.
pub fn dictionary_prompt(config: &DictationConfig) -> Option<String> {
    let mut words = config.dictionary.clone();
    words.extend(config.replacements.keys().cloned());
    let mut prompt = words
        .into_iter()
        .map(|word| word.trim().to_string())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    prompt.sort_unstable();
    prompt.dedup();
    if prompt.is_empty() {
        None
    } else {
        let mut joined = prompt.join(", ");
        if joined.len() > 1_000 {
            // A user dictionary may contain CJK or accented terms. `String::truncate`
            // panics when its byte index splits one of those code points, so
            // bound the prompt without turning a valid dictionary into a crash.
            let mut end = 1_000;
            while !joined.is_char_boundary(end) {
                end -= 1;
            }
            joined.truncate(end);
        }
        Some(joined)
    }
}

/// Transcribes the VAD-trimmed utterance and applies Layer 0 plus the optional
/// local Layer 1. A failed cleanup model never discards the usable transcript;
/// it returns the deterministic text with a visible warning for the UI.
pub fn transcribe_and_clean(
    transcriber: &dyn Transcriber,
    samples: &[f32],
    config: &DictationConfig,
) -> Result<CleanupOutcome> {
    let prompt = dictionary_prompt(config);
    let segments = transcriber
        .transcribe_with_prompt(samples, &[], prompt.as_deref())
        .context("dictation transcription")?;
    let raw = segments
        .into_iter()
        .map(|(_, _, text)| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let deterministic = cleanup::apply_replacements(
        &cleanup::layer0(&raw),
        &config.replacements,
    );
    if deterministic.is_empty() {
        anyhow::bail!("the speech model returned no words")
    }
    if !config.cleanup_enabled || !cleanup::should_run_llm(&deterministic) {
        return Ok(CleanupOutcome {
            text: deterministic,
            warning: None,
            used_llm: false,
        });
    }

    let client = LlmClient {
        base_url: config.llm_base_url.clone(),
        model: config.cleanup_model.clone(),
    };
    match cleanup::layer1(&client, &deterministic) {
        Ok(text) => Ok(CleanupOutcome {
            text: cleanup::apply_replacements(&cleanup::layer0(&text), &config.replacements),
            warning: None,
            used_llm: true,
        }),
        Err(error) => Ok(CleanupOutcome {
            text: deterministic,
            warning: Some(format!(
                "Local cleanup was unavailable; inserted the deterministic transcript ({error})"
            )),
            used_llm: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub;
    impl Transcriber for Stub {
        fn transcribe(
            &self,
            _: &[f32],
            _: &[(f32, f32)],
        ) -> Result<Vec<(f32, f32, String)>> {
            Ok(vec![(0.0, 1.0, "hello [BLANK_AUDIO] new line world".into())])
        }
    }

    #[test]
    fn dictionary_prompt_is_bounded_and_deduplicated() {
        let config = DictationConfig {
            cleanup_enabled: false,
            cleanup_model: "small".into(),
            llm_base_url: "http://127.0.0.1:1".into(),
            dictionary: vec!["Zed".into(), "zed".into()],
            replacements: [("Notetaker".into(), "Notetaker".into())]
                .into_iter()
                .collect(),
            keep_audio: false,
        };
        let prompt = dictionary_prompt(&config).unwrap();
        assert!(prompt.contains("Notetaker"));
        assert!(prompt.len() <= 1_000);
    }

    #[test]
    fn dictionary_prompt_bounds_unicode_without_splitting_a_code_point() {
        let config = DictationConfig {
            cleanup_enabled: false,
            cleanup_model: "small".into(),
            llm_base_url: "http://127.0.0.1:1".into(),
            dictionary: vec!["界".repeat(1_200)],
            replacements: BTreeMap::new(),
            keep_audio: false,
        };
        let prompt = dictionary_prompt(&config).unwrap();
        assert!(prompt.len() <= 1_000);
        assert!(prompt.is_char_boundary(prompt.len()));
        assert!(prompt.chars().all(|character| character == '界'));
    }

    #[test]
    fn prompt_aware_transcription_uses_empty_spans() {
        let config = DictationConfig {
            cleanup_enabled: false,
            cleanup_model: "small".into(),
            llm_base_url: "http://127.0.0.1:1".into(),
            dictionary: vec!["world".into()],
            replacements: BTreeMap::new(),
            keep_audio: false,
        };
        let result = transcribe_and_clean(&Stub, &[0.1; 10], &config).unwrap();
        assert_eq!(result.text, "hello\nworld");
        assert!(!result.used_llm);
    }
}
