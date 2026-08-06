//! Warm release-to-text benchmark for system-wide dictation.
//!
//! The microphone and paste event are intentionally outside this harness: the
//! clock starts after release with VAD-trimmed samples, which is the expensive
//! release-to-text portion the user experiences. The model handles are loaded
//! before the clock, matching the production lease acquired on key-press.
//!
//! Usage:
//!   cargo run -p notetaker-core --example dictation-latency -- \
//!     <audio.wav> <whisper.ggml.bin> <silero_vad.onnx> <cleanup-model>

use std::env;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use notetaker_core::dictation::{self, DictationConfig};
use notetaker_core::pipeline::audio::load_mono_16k;
use notetaker_core::pipeline::transcribe::WhisperTranscriber;

// The fixture starts with an English sentence, followed by a multilingual
// section. Keep the measured utterance inside that first sentence so the
// cleanup case exercises the documented >=8-word LLM path instead of silently
// taking the short-utterance bypass because a language switch changed token
// spacing.
const MAX_UTTERANCE_SECONDS: usize = 4;

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let [audio, whisper, vad, cleanup_model] = args.as_slice() else {
        anyhow::bail!(
            "usage: dictation-latency <audio.wav> <whisper.ggml.bin> <silero_vad.onnx> <cleanup-model>"
        );
    };

    let all_samples = load_mono_16k(Path::new(audio))?;
    let samples = all_samples
        .into_iter()
        .take(MAX_UTTERANCE_SECONDS * 16_000)
        .collect::<Vec<_>>();
    let mut gate = dictation::SileroGate::open(Path::new(vad))?;
    for chunk in samples.chunks(512) {
        gate.push(chunk);
    }
    let voiced = gate.finish();
    if voiced.is_empty() {
        anyhow::bail!("VAD found no speech in the first {MAX_UTTERANCE_SECONDS}s");
    }

    // This is the production lease's loaded handle. Each call below still
    // creates a fresh WhisperState internally.
    let transcriber = WhisperTranscriber::load(Path::new(whisper))?;
    let base = DictationConfig {
        cleanup_enabled: false,
        cleanup_model: cleanup_model.clone(),
        llm_base_url: "http://127.0.0.1:11434".into(),
        dictionary: Vec::new(),
        replacements: Default::default(),
        keep_audio: false,
    };

    let no_cleanup = timed(&transcriber, &voiced, &base)?;
    let mut with_cleanup_config = base;
    with_cleanup_config.cleanup_enabled = true;
    let with_cleanup = timed(&transcriber, &voiced, &with_cleanup_config)?;

    println!("model={cleanup_model}");
    println!("voiced_seconds={:.3}", voiced.len() as f64 / 16_000.0);
    print_result("release_to_text_without_cleanup", &no_cleanup);
    print_result("release_to_text_with_cleanup", &with_cleanup);
    Ok(())
}

fn timed(
    transcriber: &WhisperTranscriber,
    samples: &[f32],
    config: &DictationConfig,
) -> Result<(f64, String)> {
    let started = Instant::now();
    let result = dictation::transcribe_and_clean(transcriber, samples, config)
        .context("running the dictation latency case")?;
    Ok((started.elapsed().as_secs_f64(), result.text))
}

fn print_result(label: &str, result: &(f64, String)) {
    let target = if label.ends_with("without_cleanup") {
        0.8
    } else {
        1.5
    };
    println!(
        "{label}_seconds={:.3} target_seconds={target:.1} pass={}",
        result.0,
        result.0 <= target
    );
    println!("{label}_text={}", result.1.replace('\n', " "));
}
