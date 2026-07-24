//! ASR bake-off: runs each candidate speech engine on the same audio and
//! scores it against a reference transcript, so we can decide which engine
//! ships as the default for mixed English/Mandarin recordings.
//!
//!   cargo run -p notetaker-core --bin bakeoff -- \
//!       fixtures/bilingual.wav fixtures/bilingual.reference.txt \
//!       --whisper models/ggml-tiny.bin \
//!       --sensevoice models/sherpa-onnx-sense-voice-.../model.int8.onnx
//!
//! The `--sensevoice` argument is optional; if its model is absent the engine
//! is reported as skipped rather than silently dropped.

use std::path::Path;
use std::time::Instant;

use notetaker_core::pipeline::audio::load_mono_16k;
use notetaker_core::pipeline::transcribe::{Transcriber, WhisperTranscriber};

fn main() {
    if let Err(e) = run() {
        eprintln!("bakeoff error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let audio = args.next().ok_or_else(usage)?;
    let reference_path = args.next().ok_or_else(usage)?;

    let mut whisper_model = None;
    let mut sensevoice_model = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--whisper" => whisper_model = args.next(),
            "--sensevoice" => sensevoice_model = args.next(),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let samples = load_mono_16k(Path::new(&audio))?;
    let reference = std::fs::read_to_string(&reference_path)?;

    println!("audio:     {audio} ({:.1}s)", samples.len() as f32 / 16_000.0);
    println!("reference: {reference_path}\n");

    if let Some(model) = whisper_model {
        run_engine("whisper", &samples, &reference, || {
            let t = WhisperTranscriber::load(Path::new(&model))?;
            let out = t.transcribe(&samples, &[])?;
            Ok(join_text(&out))
        });
    } else {
        println!("whisper:    skipped (no --whisper model)\n");
    }

    match sensevoice_model {
        Some(model) if Path::new(&model).exists() => {
            let tokens = sibling(&model, "tokens.txt");
            run_engine("sensevoice", &samples, &reference, || {
                transcribe_sensevoice(&model, &tokens, &samples)
            });
        }
        Some(model) => println!("sensevoice: skipped (model not found at {model})\n"),
        None => println!("sensevoice: skipped (no --sensevoice model)\n"),
    }

    Ok(())
}

fn usage() -> anyhow::Error {
    anyhow::anyhow!("usage: bakeoff <audio.wav> <reference.txt> [--whisper M] [--sensevoice M]")
}

/// Runs one engine, timing it and scoring its transcript, and prints a row.
fn run_engine<F>(name: &str, _samples: &[f32], reference: &str, transcribe: F)
where
    F: FnOnce() -> anyhow::Result<String>,
{
    let start = Instant::now();
    match transcribe() {
        Ok(hypothesis) => {
            let secs = start.elapsed().as_secs_f32();
            let overall = cer(reference, &hypothesis);
            let (en_ref, zh_ref) = split_by_script(reference);
            let (en_hyp, zh_hyp) = split_by_script(&hypothesis);
            println!("=== {name} ===");
            println!("  time:        {secs:.1}s");
            println!("  CER overall: {:.1}%", overall * 100.0);
            println!("  CER english: {:.1}%", cer(&en_ref, &en_hyp) * 100.0);
            println!("  CER chinese: {:.1}%", cer(&zh_ref, &zh_hyp) * 100.0);
            println!("  transcript:  {}\n", hypothesis.trim());
        }
        Err(e) => println!("=== {name} ===\n  FAILED: {e:#}\n"),
    }
}

fn join_text(spans: &[(f32, f32, String)]) -> String {
    spans
        .iter()
        .map(|(_, _, t)| t.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

fn sibling(model: &str, name: &str) -> String {
    Path::new(model)
        .parent()
        .map(|p| p.join(name))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string())
}

fn transcribe_sensevoice(model: &str, tokens: &str, samples: &[f32]) -> anyhow::Result<String> {
    use sherpa_rs::sense_voice::{SenseVoiceConfig, SenseVoiceRecognizer};
    let mut rec = SenseVoiceRecognizer::new(SenseVoiceConfig {
        model: model.to_string(),
        tokens: tokens.to_string(),
        language: "auto".to_string(),
        ..Default::default()
    })
    .map_err(|e| anyhow::anyhow!("loading SenseVoice: {e}"))?;
    Ok(rec.transcribe(16_000, samples).text)
}

// --- scoring ---------------------------------------------------------------

/// Character error rate: Levenshtein distance over characters, normalized by
/// reference length. Case- and whitespace-normalized so cosmetic differences
/// don't dominate. Works for both scripts because it counts Unicode
/// characters, not bytes (one Han character = one unit).
fn cer(reference: &str, hypothesis: &str) -> f32 {
    let r: Vec<char> = normalize(reference).chars().collect();
    let h: Vec<char> = normalize(hypothesis).chars().collect();
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    levenshtein(&r, &h) as f32 / r.len() as f32
}

/// Lowercase, drop punctuation/whitespace so only meaningful characters are
/// compared. Keeps ASCII letters/digits and CJK ideographs.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| {
            c.is_ascii_alphanumeric() || matches!(*c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF)
        })
        .collect()
}

/// Splits text into (english, chinese) halves so each script can be scored
/// separately — "good at English, useless at Chinese" is the failure mode
/// that matters for this app.
fn split_by_script(s: &str) -> (String, String) {
    let mut en = String::new();
    let mut zh = String::new();
    for c in s.chars() {
        if matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF) {
            zh.push(c);
        } else if c.is_ascii_alphanumeric() || c.is_whitespace() {
            en.push(c);
        }
    }
    (en, zh)
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_score_zero() {
        assert_eq!(cer("hello world", "hello world"), 0.0);
        assert_eq!(cer("预算招聘计划", "预算招聘计划"), 0.0);
    }

    #[test]
    fn one_wrong_char_scores_by_reference_length() {
        // "budget" vs "budgat": 1 substitution over 6 reference chars.
        let score = cer("budget", "budgat");
        assert!((score - 1.0 / 6.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn cjk_counted_as_single_characters_not_bytes() {
        // One Han char replaced out of four: 25%, not byte-weighted.
        let score = cer("预算招聘", "预算招平");
        assert!((score - 0.25).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn split_by_script_separates_languages() {
        let (en, zh) = split_by_script("hello 大家好 world");
        assert!(en.contains("hello") && en.contains("world"));
        assert_eq!(zh, "大家好");
    }

    #[test]
    fn normalize_drops_case_and_punctuation() {
        assert_eq!(normalize("Hello, World!"), "helloworld");
    }
}
