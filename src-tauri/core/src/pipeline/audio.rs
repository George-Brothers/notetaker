//! Audio decoding: load an audio file into mono `f32` samples for the
//! transcription stage. WAV is decoded via `hound`, FLAC via `symphonia`.
//! Resampling isn't implemented yet, so callers must supply 16 kHz audio.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Loads `path` (`.wav` or `.flac`) as mono `f32` samples at 16 kHz.
///
/// Multi-channel audio is averaged down to a single channel. Integer PCM
/// samples are converted to `f32` by dividing by 32768. Errors if the file
/// isn't already 16 kHz (resampling lands with FLAC support in a later
/// task) or if the extension isn't recognized.
pub fn load_mono_16k(path: &Path) -> Result<Vec<f32>> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => load_wav(path),
        Some("flac") => load_flac(path),
        other => bail!(
            "{}: unsupported audio extension {:?} (expected .wav or .flac)",
            path.display(),
            other
        ),
    }
}

/// Averages an interleaved multi-channel buffer down to mono.
fn average_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn load_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening wav {}", path.display()))?;
    let spec = reader.spec();

    if spec.sample_rate != 16000 {
        bail!(
            "{}: expected 16 kHz audio, got {} Hz",
            path.display(),
            spec.sample_rate
        );
    }
    let channels = spec.channels as usize;
    if channels == 0 {
        bail!("{}: wav file reports zero channels", path.display());
    }

    let interleaved: Vec<f32> = reader
        .samples::<i16>()
        .map(|sample| sample.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .with_context(|| format!("reading samples from {}", path.display()))?;

    Ok(average_to_mono(&interleaved, channels))
}

fn load_flac(path: &Path) -> Result<Vec<f32>> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file =
        std::fs::File::open(path).with_context(|| format!("opening flac {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("flac");

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .with_context(|| format!("probing flac format for {}", path.display()))?;

    let track = format
        .default_track(TrackType::Audio)
        .with_context(|| format!("{}: no audio track found", path.display()))?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .with_context(|| format!("{}: missing audio codec parameters", path.display()))?;

    let sample_rate = audio_params
        .sample_rate
        .with_context(|| format!("{}: unknown sample rate", path.display()))?;
    if sample_rate != 16000 {
        bail!(
            "{}: expected 16 kHz audio, got {} Hz",
            path.display(),
            sample_rate
        );
    }

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .with_context(|| format!("{}: unsupported flac codec", path.display()))?;

    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(err) => return Err(err).context("reading flac packet"),
        };
        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let channels = audio_buf.spec().channels().count();
                let mut interleaved: Vec<f32> = Vec::new();
                audio_buf.copy_to_vec_interleaved(&mut interleaved);
                mono.extend(average_to_mono(&interleaved, channels));
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(err) => return Err(err).context("decoding flac packet"),
        }
    }

    Ok(mono)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bilingual_fixture_as_mono_16k() {
        let path = Path::new("../../fixtures/bilingual.wav");
        let samples = load_mono_16k(path).unwrap();

        // ~34.2s @ 16kHz per fixtures/README.md; allow slack for exact framing.
        let expected = (34.2 * 16000.0) as usize;
        assert!(
            samples.len() > expected - 16000 && samples.len() < expected + 16000,
            "unexpected sample count: {}",
            samples.len()
        );
        assert!(
            samples.iter().all(|s| *s >= -1.0 && *s <= 1.0),
            "samples out of [-1, 1] range"
        );
        // Real signal, not silence.
        assert!(samples.iter().any(|s| s.abs() > 0.01));
    }

    #[test]
    fn rejects_unsupported_extension() {
        let err = load_mono_16k(Path::new("nope.mp3")).unwrap_err();
        assert!(err.to_string().contains("unsupported audio extension"));
    }
}
