//! The download allowlist. These are the ONLY URLs the shipped app will
//! ever fetch a model from — every entry's URL and sha256 was verified by
//! hand against the authoritative source (the whisper.cpp Hugging Face repo
//! and the k2-fsa/sherpa-onnx GitHub release assets) at implementation
//! time. See each constant's doc comment for provenance.

use super::ModelSpec;

/// Whisper `large-v3-turbo`, full precision ggml. Used on the
/// `AppleSiliconBig` and `CpuBig` tiers.
///
/// Source: <https://huggingface.co/ggerganov/whisper.cpp>
/// sha256 verified against the repo's published LFS object id (also
/// confirmed via the `x-linked-etag` response header on the download URL).
pub const WHISPER_LARGE_V3_TURBO: ModelSpec = ModelSpec {
    name: "whisper-large-v3-turbo",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    dest: "ggml-large-v3-turbo.bin",
};

/// Whisper `small`, q5_1 quantized ggml. Used on the `AppleSiliconSmall` and
/// `CpuSmall` tiers.
///
/// Source: <https://huggingface.co/ggerganov/whisper.cpp>
/// sha256 verified against the repo's published LFS object id (also
/// confirmed via the `x-linked-etag` response header on the download URL).
pub const WHISPER_SMALL_Q5_1: ModelSpec = ModelSpec {
    name: "whisper-small-q5_1",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
    sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    dest: "ggml-small-q5_1.bin",
};

/// sherpa-onnx packaging of pyannote `segmentation-3.0` (a tarball
/// containing `model.onnx` plus license/export scripts — extraction happens
/// wherever this is loaded, not here). Used on all tiers.
///
/// Source: <https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-segmentation-models>
/// GitHub does not publish a digest for this asset, so the sha256 below was
/// computed directly from a fresh download of the release asset (not
/// invented, not copied from a third party).
pub const DIARIZATION_SEGMENTATION: ModelSpec = ModelSpec {
    name: "diarization-segmentation",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
    sha256: "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488",
    dest: "sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
};

/// sherpa-onnx WeSpeaker `voxceleb-resnet34-LM` speaker-embedding model —
/// the same embedding extractor pyannote's own reference pipeline pairs
/// with `segmentation-3.0`. Used on all tiers.
///
/// Source: <https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models>
/// sha256 verified two ways: matches the maintainer-published
/// `checksum.txt` in that release, and matches an independent sha256 of a
/// fresh download performed during implementation.
pub const DIARIZATION_EMBEDDING: ModelSpec = ModelSpec {
    name: "diarization-embedding",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx",
    sha256: "e9848563da86f263117134dfd7ad63c92355b37de492b55e325400c9d9c39012",
    dest: "wespeaker_en_voxceleb_resnet34_LM.onnx",
};

const REGISTRY: &[ModelSpec] = &[
    WHISPER_LARGE_V3_TURBO,
    WHISPER_SMALL_Q5_1,
    DIARIZATION_SEGMENTATION,
    DIARIZATION_EMBEDDING,
];

/// The full allowlist — every model the app is ever allowed to download.
pub fn all() -> &'static [ModelSpec] {
    REGISTRY
}

/// The models required for a given hardware tier: the tier-appropriate
/// Whisper model plus the two (tier-independent) diarization models.
pub fn required_models(tier: &super::Tier) -> Vec<&'static ModelSpec> {
    use super::Tier::*;
    match tier {
        AppleSiliconBig | CpuBig => vec![
            &WHISPER_LARGE_V3_TURBO,
            &DIARIZATION_SEGMENTATION,
            &DIARIZATION_EMBEDDING,
        ],
        AppleSiliconSmall | CpuSmall => vec![
            &WHISPER_SMALL_Q5_1,
            &DIARIZATION_SEGMENTATION,
            &DIARIZATION_EMBEDDING,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Tier;

    #[test]
    fn all_returns_all_four_registry_entries() {
        assert_eq!(all().len(), 4);
    }

    #[test]
    fn apple_silicon_big_gets_large_turbo_and_diarization() {
        let names: Vec<&str> = required_models(&Tier::AppleSiliconBig)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "whisper-large-v3-turbo",
                "diarization-segmentation",
                "diarization-embedding",
            ]
        );
    }

    #[test]
    fn apple_silicon_small_gets_small_q5_and_diarization() {
        let names: Vec<&str> = required_models(&Tier::AppleSiliconSmall)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "whisper-small-q5_1",
                "diarization-segmentation",
                "diarization-embedding",
            ]
        );
    }

    /// A desktop-class Windows or Intel machine gets the same models as a big
    /// Mac. Pinned as an equality against `AppleSiliconBig` rather than a
    /// literal list, so adding a model to the big tier can never leave the
    /// CPU-big tier silently behind.
    #[test]
    fn cpu_big_matches_apple_silicon_big() {
        let cpu: Vec<&str> = required_models(&Tier::CpuBig)
            .iter()
            .map(|m| m.name)
            .collect();
        let apple_big: Vec<&str> = required_models(&Tier::AppleSiliconBig)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(cpu, apple_big);
    }

    /// Every tier must resolve to a non-empty model set. An exhaustive match
    /// makes adding a `Tier` variant a compile error here, not a runtime
    /// surprise where first-run downloads nothing and transcription silently
    /// never starts.
    #[test]
    fn every_tier_requires_at_least_one_speech_model() {
        for tier in [
            Tier::AppleSiliconBig,
            Tier::AppleSiliconSmall,
            Tier::CpuBig,
            Tier::CpuSmall,
        ] {
            let models = required_models(&tier);
            assert!(
                models.iter().any(|m| m.name.starts_with("whisper")),
                "{tier:?} has no speech model"
            );
        }
    }

    #[test]
    fn cpu_small_matches_apple_silicon_small() {
        let cpu: Vec<&str> = required_models(&Tier::CpuSmall)
            .iter()
            .map(|m| m.name)
            .collect();
        let apple_small: Vec<&str> = required_models(&Tier::AppleSiliconSmall)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(cpu, apple_small);
    }

    #[test]
    fn every_registry_entry_uses_https() {
        for spec in all() {
            assert!(
                spec.url.starts_with("https://"),
                "{} has a non-https url: {}",
                spec.name,
                spec.url
            );
        }
    }

    #[test]
    fn every_registry_hash_is_empty_or_64_lowercase_hex_chars() {
        for spec in all() {
            let h = spec.sha256;
            assert!(
                h.is_empty()
                    || (h.len() == 64
                        && h.chars()
                            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())),
                "{} has a malformed sha256: {:?}",
                spec.name,
                h
            );
        }
    }
}
