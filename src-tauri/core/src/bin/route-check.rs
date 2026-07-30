//! Runs a real recording through Whisper alone and through the router, and
//! prints both, so the routing decision can be judged on real audio rather
//! than asserted about with fakes.
//!
//! Not a test: it needs models and a real file, neither of which belong in the
//! suite. It is the tool that answers "is this actually better?".
//!
//! ```text
//! route-check <audio-16k-mono.wav> \
//!     --whisper models/ggml-small-q5_1.bin \
//!     --sense-voice models/model.int8.onnx \
//!     --tokens models/tokens.txt \
//!     --segmentation models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx \
//!     --embedding models/wespeaker_en_voxceleb_resnet34_LM.onnx
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use notetaker_core::api::SpeechEngine;
use notetaker_core::pipeline::audio::load_mono_16k;
use notetaker_core::pipeline::diarize::{Diarizer, SherpaDiarizer};
use notetaker_core::pipeline::route::{LanguageTranscriber, RoutingTranscriber};
use notetaker_core::pipeline::transcribe::{
    SenseVoiceTranscriber, Transcriber, WhisperTranscriber,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let audio = args.next().ok_or_else(usage)?;
    let mut whisper_path = None;
    let mut sense_voice_path = None;
    let mut tokens_path = None;
    let mut segmentation_path = None;
    let mut embedding_path = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--whisper" => whisper_path = args.next().map(PathBuf::from),
            "--sense-voice" => sense_voice_path = args.next().map(PathBuf::from),
            "--tokens" => tokens_path = args.next().map(PathBuf::from),
            "--segmentation" => segmentation_path = args.next().map(PathBuf::from),
            "--embedding" => embedding_path = args.next().map(PathBuf::from),
            other => anyhow::bail!("unknown flag {other}"),
        }
    }

    let samples = load_mono_16k(Path::new(&audio))?;
    println!(
        "audio: {} ({:.1}s)\n",
        audio,
        samples.len() as f32 / 16000.0
    );

    let spans = match (segmentation_path, embedding_path) {
        (Some(seg), Some(emb)) => {
            let started = Instant::now();
            let diarizer = SherpaDiarizer::load(&seg, &emb)?;
            let speaker_spans = diarizer.diarize(&samples)?;
            println!(
                "diarization: {} segments in {:.1}s",
                speaker_spans.len(),
                started.elapsed().as_secs_f32()
            );
            speaker_spans
                .iter()
                .map(|s| (s.start_s, s.end_s))
                .collect::<Vec<_>>()
        }
        _ => {
            println!("diarization: skipped (no models given)");
            Vec::new()
        }
    };

    let whisper = WhisperTranscriber::load(&whisper_path.ok_or_else(usage)?)?;

    let started = Instant::now();
    let whisper_only = whisper.transcribe(&samples, &spans)?;
    let whisper_secs = started.elapsed().as_secs_f32();
    report("WHISPER ONLY", &whisper_only, whisper_secs);

    let (Some(model), Some(tokens)) = (sense_voice_path, tokens_path) else {
        println!("\n(no SenseVoice given — nothing to compare against)");
        return Ok(());
    };
    let sense_voice: Box<dyn LanguageTranscriber + Send + Sync> =
        Box::new(SenseVoiceTranscriber::load(&model, &tokens)?);
    let router = RoutingTranscriber::new(Box::new(whisper), Some(sense_voice), SpeechEngine::Auto);

    let started = Instant::now();
    let routed = router.transcribe(&samples, &spans)?;
    let routed_secs = started.elapsed().as_secs_f32();
    report("ROUTED (SenseVoice + Whisper)", &routed, routed_secs);

    println!(
        "\nspeed: whisper-only {whisper_secs:.1}s vs routed {routed_secs:.1}s ({:.2}x)",
        routed_secs / whisper_secs.max(0.001)
    );
    Ok(())
}

fn report(label: &str, out: &[(f32, f32, String)], secs: f32) {
    println!("\n=== {label} — {} segments, {secs:.1}s ===", out.len());
    for (start, _, text) in out.iter().take(24) {
        println!("  [{start:7.2}] {text}");
    }
    if out.len() > 24 {
        println!("  … {} more", out.len() - 24);
    }
    let all: String = out.iter().map(|(_, _, t)| t.as_str()).collect();
    let cjk = all
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    println!("  chars: {} of which CJK: {cjk}", all.chars().count());
}

fn usage() -> anyhow::Error {
    anyhow::anyhow!(
        "usage: route-check <audio.wav> --whisper M [--sense-voice M --tokens T] \
         [--segmentation S --embedding E]"
    )
}
