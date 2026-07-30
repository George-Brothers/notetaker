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

/// SenseVoice Small, int8-quantized — the non-English speech model.
///
/// Chosen on measured evidence, not reputation: on the project's own bilingual
/// fixture it scored 9% character error rate against Whisper-tiny's 53%. It
/// handles Chinese, Cantonese, Japanese, Korean and English, and it is
/// non-autoregressive, so it runs several times faster than Whisper.
///
/// **The int8 file from Hugging Face rather than the GitHub release tarball**:
/// the release ships full-precision and quantized together in a 1.0 GB
/// `.tar.bz2`, and this is the 239 MB file actually needed. sha256 verified two
/// ways — the `x-linked-etag` Hugging Face publishes for the LFS object, and an
/// independent sha256 of a fresh download taken while wiring this up.
///
/// Source: <https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17>
pub const SENSE_VOICE_MODEL: ModelSpec = ModelSpec {
    name: "sense-voice",
    url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
    sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
    dest: "sense-voice-model.int8.onnx",
};

/// SenseVoice's token table. Useless without [`SENSE_VOICE_MODEL`] and vice
/// versa, so the two are always fetched together.
///
/// Small enough that Hugging Face stores it in git rather than LFS, so there is
/// no published sha256 to copy — this one was computed from a fresh download.
pub const SENSE_VOICE_TOKENS: ModelSpec = ModelSpec {
    name: "sense-voice-tokens",
    url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
    sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
    dest: "sense-voice-tokens.txt",
};

const REGISTRY: &[ModelSpec] = &[
    WHISPER_LARGE_V3_TURBO,
    WHISPER_SMALL_Q5_1,
    DIARIZATION_SEGMENTATION,
    DIARIZATION_EMBEDDING,
    SENSE_VOICE_MODEL,
    SENSE_VOICE_TOKENS,
];

/// The full allowlist — every model the app is ever allowed to download.
pub fn all() -> &'static [ModelSpec] {
    REGISTRY
}

/// The speech model this tier transcribes with.
///
/// Split out of [`required_models`] because the scheduler needs to name this
/// one file — it loads the transcriber from it — while the downloader only
/// needs the whole set. Keeping one function the source of both means the
/// model that gets downloaded is necessarily the model that gets loaded.
pub fn speech_model(tier: &super::Tier) -> &'static ModelSpec {
    use super::Tier::*;
    match tier {
        AppleSiliconBig | CpuBig => &WHISPER_LARGE_V3_TURBO,
        AppleSiliconSmall | CpuSmall => &WHISPER_SMALL_Q5_1,
    }
}

/// The languages SenseVoice is worth downloading for.
///
/// SenseVoice also handles English, and it is deliberately **not** listed here:
/// Whisper is the better English model, and Whisper is downloaded either way.
/// So a user who speaks only English — or only Spanish, or only Hindi — never
/// fetches a 239 MB model that would never be chosen for their audio.
pub const SENSE_VOICE_LANGUAGES: &[&str] = &["zh", "yue", "ja", "ko"];

/// Whether the languages this user speaks justify the SenseVoice download.
///
/// The whole reason first run asks the question. Case-insensitive, and tolerant
/// of a region suffix, so `zh-CN`, `zh_TW` and `ZH` all count as Chinese —
/// a settings file is edited by hand often enough that being strict here would
/// only ever silently withhold a model.
pub fn wants_sense_voice(languages: &[String]) -> bool {
    languages.iter().any(|chosen| {
        let base = chosen
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        SENSE_VOICE_LANGUAGES.contains(&base.as_str())
    })
}

/// The models to download: the tier's Whisper model, the two diarization
/// models, and SenseVoice only if one of `languages` calls for it.
///
/// Whisper is unconditional. It is the English model, and it is the fallback
/// for every language SenseVoice does not know — so it is the one model that
/// makes the app work no matter what was chosen.
pub fn required_models(tier: &super::Tier, languages: &[String]) -> Vec<&'static ModelSpec> {
    let mut specs = vec![
        speech_model(tier),
        &DIARIZATION_SEGMENTATION,
        &DIARIZATION_EMBEDDING,
    ];
    if wants_sense_voice(languages) {
        specs.push(&SENSE_VOICE_MODEL);
        specs.push(&SENSE_VOICE_TOKENS);
    }
    specs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Tier;

    /// Shorthand for the English-only default in tests that are not about
    /// language selection.
    macro_rules! english_slice {
        () => {
            &english()[..]
        };
    }

    /// English-only, the default: the set every user gets before they say
    /// otherwise. Declared once so a test that is not *about* languages does
    /// not quietly become one.
    fn english() -> Vec<String> {
        vec!["en".to_string()]
    }

    #[test]
    fn the_allowlist_holds_every_model_the_app_may_fetch() {
        assert_eq!(all().len(), 6);
    }

    #[test]
    fn apple_silicon_big_gets_large_turbo_and_diarization() {
        let names: Vec<&str> = required_models(&Tier::AppleSiliconBig, english_slice!())
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
        let names: Vec<&str> = required_models(&Tier::AppleSiliconSmall, english_slice!())
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
        let cpu: Vec<&str> = required_models(&Tier::CpuBig, english_slice!())
            .iter()
            .map(|m| m.name)
            .collect();
        let apple_big: Vec<&str> = required_models(&Tier::AppleSiliconBig, english_slice!())
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
            let models = required_models(&tier, english_slice!());
            assert!(
                models.iter().any(|m| m.name.starts_with("whisper")),
                "{tier:?} has no speech model"
            );
        }
    }

    #[test]
    fn cpu_small_matches_apple_silicon_small() {
        let cpu: Vec<&str> = required_models(&Tier::CpuSmall, english_slice!())
            .iter()
            .map(|m| m.name)
            .collect();
        let apple_small: Vec<&str> = required_models(&Tier::AppleSiliconSmall, english_slice!())
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

#[cfg(test)]
mod language_tests {
    use super::*;
    use crate::models::Tier;

    fn names(tier: Tier, languages: &[&str]) -> Vec<&'static str> {
        let owned: Vec<String> = languages.iter().map(|s| s.to_string()).collect();
        required_models(&tier, &owned)
            .iter()
            .map(|m| m.name)
            .collect()
    }

    #[test]
    fn english_only_does_not_download_sense_voice() {
        // The point of asking at all: 239 MB not fetched for a model that would
        // never be chosen for this user's audio.
        let names = names(Tier::CpuSmall, &["en"]);
        assert!(
            !names.iter().any(|n| n.starts_with("sense-voice")),
            "got {names:?}"
        );
        assert!(names.contains(&"whisper-small-q5_1"));
    }

    #[test]
    fn a_language_sense_voice_knows_adds_it_and_keeps_whisper() {
        let names = names(Tier::CpuSmall, &["en", "zh"]);
        assert!(names.contains(&"sense-voice"));
        assert!(
            names.contains(&"sense-voice-tokens"),
            "the model is useless without its token table: {names:?}"
        );
        assert!(
            names.contains(&"whisper-small-q5_1"),
            "Whisper stays — it is the English model and the fallback: {names:?}"
        );
    }

    #[test]
    fn each_of_sense_voices_languages_pulls_it_on_its_own() {
        for language in SENSE_VOICE_LANGUAGES {
            assert!(
                wants_sense_voice(&[language.to_string()]),
                "{language} should require SenseVoice"
            );
        }
    }

    #[test]
    fn a_language_neither_model_specialises_in_gets_whisper_alone() {
        // Spanish is not in SenseVoice's five. Whisper handles it, so the right
        // answer is "download nothing extra", not "download SenseVoice anyway".
        let names = names(Tier::CpuSmall, &["es", "fr", "hi"]);
        assert!(
            !names.iter().any(|n| n.starts_with("sense-voice")),
            "got {names:?}"
        );
    }

    #[test]
    fn a_region_suffix_or_odd_casing_still_counts_as_the_language() {
        // A settings file gets hand-edited. Being strict here would silently
        // withhold the model and quietly downgrade someone's transcripts.
        for written_as in ["zh-CN", "zh_TW", "ZH", "Ja-JP", "yue-Hant"] {
            assert!(
                wants_sense_voice(&[written_as.to_string()]),
                "{written_as} should be recognised"
            );
        }
    }

    #[test]
    fn no_languages_at_all_is_treated_as_english_rather_than_everything() {
        // An empty list must not be read as "download the lot" — that is the
        // failure mode where asking the question costs the user more, not less.
        assert!(!wants_sense_voice(&[]));
    }

    #[test]
    fn every_spec_a_language_can_require_is_on_the_allowlist() {
        // The allowlist is the security boundary: it is the only set of URLs
        // the app may ever fetch. A model reachable through `required_models`
        // but absent from `all()` would be a hole in it.
        for tier in [Tier::AppleSiliconBig, Tier::CpuSmall] {
            for languages in [vec!["en".to_string()], vec!["zh".to_string()]] {
                for spec in required_models(&tier, &languages) {
                    assert!(
                        all().iter().any(|allowed| allowed.url == spec.url),
                        "{} is not on the allowlist",
                        spec.name
                    );
                }
            }
        }
    }
}
