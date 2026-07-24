//! Speaker diarization: who spoke when.

/// A stretch of audio attributed to one (0-based, per-recording) speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeakerSpan {
    pub start_s: f32,
    pub end_s: f32,
    pub speaker: u32,
}

/// Something that can split a recording's samples into per-speaker spans.
pub trait Diarizer {
    fn diarize(&self, samples: &[f32]) -> anyhow::Result<Vec<SpeakerSpan>>;
}

/// Offline speaker diarization backed by sherpa-onnx: pyannote segmentation
/// plus a speaker-embedding model, clustered by cosine-distance threshold
/// since the true speaker count is not known ahead of time.
pub struct SherpaDiarizer {
    inner: std::sync::Mutex<sherpa_rs::diarize::Diarize>,
}

impl SherpaDiarizer {
    pub fn load(
        segmentation_onnx: &std::path::Path,
        embedding_onnx: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let config = sherpa_rs::diarize::DiarizeConfig {
            // Speaker count is unknown up front, so cluster by distance
            // threshold rather than forcing a fixed number of speakers
            // (num_clusters <= 0 tells sherpa-onnx to use `threshold`).
            num_clusters: Some(-1),
            threshold: Some(0.5),
            ..Default::default()
        };
        let inner = sherpa_rs::diarize::Diarize::new(segmentation_onnx, embedding_onnx, config)
            .map_err(|e| anyhow::anyhow!("failed to load sherpa-onnx diarizer: {e}"))?;
        Ok(Self {
            inner: std::sync::Mutex::new(inner),
        })
    }
}

impl Diarizer for SherpaDiarizer {
    fn diarize(&self, samples: &[f32]) -> anyhow::Result<Vec<SpeakerSpan>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("diarizer lock poisoned"))?;
        let segments = inner
            .compute(samples.to_vec(), None)
            .map_err(|e| anyhow::anyhow!("diarization failed: {e}"))?;
        Ok(segments
            .into_iter()
            .map(|s| SpeakerSpan {
                start_s: s.start,
                end_s: s.end,
                // sherpa-onnx speaker ids are non-negative in practice for
                // real (non-noise) segments; clamp defensively rather than
                // panic on cast.
                speaker: s.speaker.max(0) as u32,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode a 16kHz mono 16-bit PCM WAV into `f32` samples in [-1, 1].
    /// Stand-in for the not-yet-written `pipeline::audio::load_mono_16k`
    /// (a parallel task owns that function); this test decodes the fixture
    /// itself with `hound` so it does not depend on unfinished work.
    fn load_wav_mono_16k(path: &std::path::Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path)
            .unwrap_or_else(|e| panic!("open fixture wav {path:?}: {e}"));
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000, "fixture must be 16kHz");
        assert_eq!(spec.channels, 1, "fixture must be mono");
        reader
            .samples::<i16>()
            .map(|s| s.expect("decode pcm sample") as f32 / i16::MAX as f32)
            .collect()
    }

    /// Path helper anchored on `CARGO_MANIFEST_DIR` (this crate's own
    /// directory, `src-tauri/core`) so the test doesn't depend on cargo's
    /// working-directory choice for test binaries.
    fn repo_path(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    #[test]
    fn fixture_yields_at_least_two_speakers() {
        let seg_path = repo_path("../../models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx");
        let emb_path =
            repo_path("../../models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx");
        if !seg_path.exists() || !emb_path.exists() {
            eprintln!("SKIP: run scripts/fetch-diarization-models.sh");
            return;
        }

        let samples = load_wav_mono_16k(&repo_path("../../fixtures/bilingual.wav"));
        let d = SherpaDiarizer::load(&seg_path, &emb_path).unwrap();
        let spans = d.diarize(&samples).unwrap();

        let speakers: std::collections::BTreeSet<u32> = spans.iter().map(|s| s.speaker).collect();
        assert!(
            speakers.len() >= 2,
            "expected >=2 speakers, got {:?} from spans {:?}",
            speakers,
            spans
        );
        assert!(
            spans.iter().all(|s| s.end_s > s.start_s),
            "every span must have end_s > start_s: {:?}",
            spans
        );
    }
}
