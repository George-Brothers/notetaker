//! Local speech-to-text via whisper.cpp (through `whisper-rs`), with
//! automatic English/Mandarin language detection.

use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const SAMPLE_RATE: f32 = 16000.0;

/// Turns audio samples into timestamped text. Implementations run entirely
/// locally (no network calls).
pub trait Transcriber {
    /// Transcribes `samples` (mono `f32` @ 16 kHz).
    ///
    /// `spans` are absolute `(start_s, end_s)` time ranges to transcribe
    /// individually (e.g. from diarization); an empty slice transcribes the
    /// whole file as one span. Returns `(start_s, end_s, text)` tuples with
    /// offsets re-based to the original (full) audio.
    fn transcribe(&self, samples: &[f32], spans: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>>;
}

/// A `Transcriber` backed by a local whisper.cpp model via `whisper-rs`.
pub struct WhisperTranscriber {
    ctx: WhisperContext,
}

impl WhisperTranscriber {
    /// Loads a ggml whisper model (e.g. `ggml-tiny.bin`) from disk.
    pub fn load(model_path: &Path) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .with_context(|| format!("loading whisper model {}", model_path.display()))?;
        Ok(Self { ctx })
    }

    /// Runs whisper on one contiguous span of samples, re-basing the
    /// resulting segment timestamps by `offset_s` so they're absolute
    /// w.r.t. the original audio.
    fn transcribe_span(&self, span_samples: &[f32], offset_s: f32) -> Result<Vec<(f32, f32, String)>> {
        let mut state = self.ctx.create_state().context("creating whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // Auto-detect language so English and Mandarin both come out in
        // their own language, never translated to English.
        params.set_language(None);
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, span_samples)
            .context("running whisper inference")?;

        let mut out = Vec::new();
        for segment in state.as_iter() {
            let text = segment
                .to_str()
                .context("decoding whisper segment text")?
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            let start_s = offset_s + segment.start_timestamp() as f32 / 100.0;
            let end_s = offset_s + segment.end_timestamp() as f32 / 100.0;
            out.push((start_s, end_s, text));
        }
        Ok(out)
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, samples: &[f32], spans: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>> {
        if spans.is_empty() {
            return self.transcribe_span(samples, 0.0);
        }

        let mut out = Vec::new();
        for &(start_s, end_s) in spans {
            let start_idx = ((start_s * SAMPLE_RATE).round() as usize).min(samples.len());
            let end_idx = ((end_s * SAMPLE_RATE).round() as usize).min(samples.len());
            if end_idx <= start_idx {
                continue;
            }
            out.extend(self.transcribe_span(&samples[start_idx..end_idx], start_s)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribes_fixture_in_two_languages() {
        let model = Path::new("../../models/ggml-tiny.bin"); // multilingual tiny, ~75MB
        if !model.exists() {
            eprintln!("SKIP: run scripts/fetch-whisper-model.sh");
            return;
        }
        let samples =
            crate::pipeline::audio::load_mono_16k(Path::new("../../fixtures/bilingual.wav"))
                .unwrap();
        let t = WhisperTranscriber::load(model).unwrap();
        let out = t.transcribe(&samples, &[]).unwrap();
        let all: String = out.iter().map(|(_, _, s)| s.as_str()).collect::<Vec<_>>().join(" ");
        assert!(all.to_lowercase().contains("budget"), "english missing: {all}");
        assert!(
            all.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "chinese missing: {all}"
        );
        assert!(out.len() >= 2, "expected at least 2 segments, got {}: {all}", out.len());
    }

    #[test]
    fn spans_rebase_offsets_to_absolute_time() {
        let model = Path::new("../../models/ggml-tiny.bin");
        if !model.exists() {
            eprintln!("SKIP: run scripts/fetch-whisper-model.sh");
            return;
        }
        let samples =
            crate::pipeline::audio::load_mono_16k(Path::new("../../fixtures/bilingual.wav"))
                .unwrap();
        let t = WhisperTranscriber::load(model).unwrap();

        // Speaker B's first Mandarin turn falls in roughly this window (see
        // fixtures/README.md); starting well after t=0 makes a wrongly
        // zeroed offset obvious.
        let out = t.transcribe(&samples, &[(3.0, 12.5)]).unwrap();

        assert!(!out.is_empty(), "expected at least one segment in the span");
        for (start_s, end_s, _) in &out {
            assert!(
                *start_s >= 3.0 - 0.5,
                "segment start {start_s} not rebased to span offset"
            );
            assert!(*end_s <= 12.5 + 0.5, "segment end {end_s} exceeds span");
        }
        let all: String = out.iter().map(|(_, _, s)| s.as_str()).collect::<Vec<_>>().join(" ");
        assert!(
            all.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "chinese missing from span transcript: {all}"
        );
    }
}
