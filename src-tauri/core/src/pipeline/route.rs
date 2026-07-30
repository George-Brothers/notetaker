//! Choosing a speech model per segment, so a bilingual recording gets the
//! better model for each part of it.
//!
//! The measured problem: on the project's own bilingual fixture, Whisper-tiny
//! scored a 53% character error rate on the Chinese and SenseVoice scored 9%.
//! Whisper is the better English model. Neither is better at *both*, and a
//! recording where two people switch language mid-conversation gets whichever
//! one it was given.
//!
//! **The trick that makes this cheap: SenseVoice reports the language it
//! detected, in the same call that returns the text.** So there is no separate
//! language-detection pass to pay for. Each segment goes through SenseVoice
//! once; if what came back is English, that segment is re-run through Whisper
//! and Whisper's text is kept. Everything else keeps SenseVoice's.
//!
//! SenseVoice is non-autoregressive and several times faster than Whisper, so
//! for a Chinese-heavy recording this is *faster* than transcribing it with
//! Whisper alone, not slower.
//!
//! Segments come from diarization, which the pipeline already runs to work out
//! who is speaking. Routing therefore adds no segmentation of its own — it
//! reuses boundaries that are already there, and those boundaries fall at
//! speaker changes, which is exactly where a language change usually happens.

use anyhow::Result;

use super::transcribe::Transcriber;
use crate::api::SpeechEngine;

/// The language tag SenseVoice returns for English.
///
/// Compared after [`normalize_lang`] strips the `<|…|>` wrapper sherpa-onnx
/// puts around it.
const ENGLISH: &str = "en";

/// Sends each segment to whichever loaded model is better for its language.
///
/// `sense_voice` is `None` when the user speaks no language it would win at —
/// then this is exactly [`WhisperTranscriber`] with one extra indirection, and
/// no second model is loaded or downloaded.
///
/// [`WhisperTranscriber`]: super::transcribe::WhisperTranscriber
pub struct RoutingTranscriber {
    whisper: Box<dyn Transcriber + Send + Sync>,
    sense_voice: Option<Box<dyn LanguageTranscriber + Send + Sync>>,
    engine: SpeechEngine,
}

/// A transcriber that also reports which language it heard.
///
/// Separate from [`Transcriber`] because only SenseVoice can do this in the
/// same pass, and that property is the whole reason routing is affordable. A
/// trait rather than the concrete type so the routing logic can be tested
/// without loading a 239 MB model.
pub trait LanguageTranscriber {
    /// Transcribes one contiguous run of samples, returning
    /// `(language, text)`. The language is a bare code such as `"en"` or
    /// `"zh"`; an empty string means "could not tell".
    fn transcribe_span(&self, samples: &[f32]) -> Result<(String, String)>;
}

impl RoutingTranscriber {
    pub fn new(
        whisper: Box<dyn Transcriber + Send + Sync>,
        sense_voice: Option<Box<dyn LanguageTranscriber + Send + Sync>>,
        engine: SpeechEngine,
    ) -> Self {
        RoutingTranscriber {
            whisper,
            sense_voice,
            engine,
        }
    }

    /// Whether this instance will actually route, as opposed to sending
    /// everything to one model. Used for the log line that tells a user which
    /// of the two they are getting.
    pub fn routes(&self) -> bool {
        self.sense_voice.is_some() && self.engine == SpeechEngine::Auto
    }
}

impl Transcriber for RoutingTranscriber {
    fn transcribe(&self, samples: &[f32], spans: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>> {
        let Some(sense_voice) = self.sense_voice.as_deref() else {
            // Nothing to route to. The overwhelmingly common case for an
            // English-only user, and identical to the behaviour before routing
            // existed.
            return self.whisper.transcribe(samples, spans);
        };

        match self.engine {
            SpeechEngine::Whisper => return self.whisper.transcribe(samples, spans),
            SpeechEngine::Auto | SpeechEngine::SenseVoice => {}
        }

        if spans.is_empty() {
            // Without spans there is nothing to route *between*, and SenseVoice
            // returns one untimed blob for however much audio it is handed —
            // which for an hour-long recording would be a transcript with a
            // single timestamp on it. Whisper segments internally, so it is the
            // right answer here even for Chinese.
            return self.whisper.transcribe(samples, spans);
        }

        let mut out = Vec::with_capacity(spans.len());
        for &(start_s, end_s) in spans {
            let Some(slice) = slice_for(samples, start_s, end_s) else {
                continue;
            };

            let (lang, text) = sense_voice.transcribe_span(slice)?;
            let text = if self.engine == SpeechEngine::Auto && is_english(&lang) {
                // Whisper is the better English model, so this segment is worth
                // a second pass. `transcribe` rather than a raw span call so
                // Whisper's own internal segmentation and timestamps are used
                // for it, exactly as they would be without routing.
                let refined = self.whisper.transcribe(samples, &[(start_s, end_s)])?;
                if refined.is_empty() {
                    // Whisper heard nothing where SenseVoice heard something.
                    // Keeping SenseVoice's text loses nothing and avoids
                    // silently dropping speech.
                    text
                } else {
                    out.extend(refined);
                    continue;
                }
            } else {
                text
            };

            if !text.trim().is_empty() {
                out.push((start_s, end_s, text.trim().to_string()));
            }
        }
        Ok(out)
    }
}

/// The samples covered by `start_s..end_s`, or `None` when that range is empty
/// or entirely past the end of the audio.
fn slice_for(samples: &[f32], start_s: f32, end_s: f32) -> Option<&[f32]> {
    const SAMPLE_RATE: f32 = 16000.0;
    let start = ((start_s * SAMPLE_RATE).round().max(0.0) as usize).min(samples.len());
    let end = ((end_s * SAMPLE_RATE).round().max(0.0) as usize).min(samples.len());
    (end > start).then(|| &samples[start..end])
}

/// Strips sherpa-onnx's `<|zh|>` wrapper and lowercases, leaving a bare code.
///
/// The wrapper is not documented as stable, and a bare `zh` is equally
/// plausible from a future version, so both are accepted rather than one being
/// assumed.
pub fn normalize_lang(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("<|")
        .trim_end_matches("|>")
        .to_ascii_lowercase()
}

/// Whether a detected language should be re-run through Whisper.
///
/// An *unrecognised or empty* tag is deliberately **not** treated as English.
/// SenseVoice already produced text for it; sending it to Whisper on a guess
/// would be spending time to replace a real transcript with one from a model
/// that was not chosen for that language.
fn is_english(raw: &str) -> bool {
    normalize_lang(raw) == ENGLISH
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Asked = Arc<Mutex<Vec<(f32, f32)>>>;

    /// Records which spans it was asked about, so a test can prove routing sent
    /// the right work to the right model rather than merely producing text.
    struct SpyWhisper {
        asked: Asked,
        reply: &'static str,
    }

    impl SpyWhisper {
        fn new(reply: &'static str) -> Self {
            SpyWhisper {
                asked: Asked::default(),
                reply,
            }
        }

        /// A spy plus the handle a test reads its record through, so the
        /// transcriber can be moved into the router and still be observed.
        fn watched(reply: &'static str) -> (Self, Asked) {
            let spy = Self::new(reply);
            let asked = Arc::clone(&spy.asked);
            (spy, asked)
        }
    }

    impl Transcriber for SpyWhisper {
        fn transcribe(
            &self,
            _samples: &[f32],
            spans: &[(f32, f32)],
        ) -> Result<Vec<(f32, f32, String)>> {
            let mut asked = self.asked.lock().unwrap();
            if spans.is_empty() {
                asked.push((-1.0, -1.0));
                return Ok(vec![(0.0, 1.0, self.reply.to_string())]);
            }
            asked.extend_from_slice(spans);
            Ok(spans
                .iter()
                .map(|&(s, e)| (s, e, self.reply.to_string()))
                .collect())
        }
    }

    /// Answers with a language per call, in order, so one recording can switch
    /// language partway through exactly as a real bilingual meeting does.
    struct ScriptedSenseVoice {
        script: Mutex<Vec<(&'static str, &'static str)>>,
        calls: Mutex<usize>,
    }

    impl ScriptedSenseVoice {
        fn new(script: Vec<(&'static str, &'static str)>) -> Self {
            ScriptedSenseVoice {
                script: Mutex::new(script),
                calls: Mutex::new(0),
            }
        }
    }

    impl LanguageTranscriber for ScriptedSenseVoice {
        fn transcribe_span(&self, _samples: &[f32]) -> Result<(String, String)> {
            let mut calls = self.calls.lock().unwrap();
            let script = self.script.lock().unwrap();
            let (lang, text) = script[*calls % script.len()];
            *calls += 1;
            Ok((lang.to_string(), text.to_string()))
        }
    }

    fn audio(seconds: f32) -> Vec<f32> {
        vec![0.1; (seconds * 16000.0) as usize]
    }

    #[test]
    fn a_bilingual_recording_uses_both_models_at_different_times() {
        // The feature, in one test: Chinese stays with SenseVoice, English is
        // re-run through Whisper, in a single recording.
        let whisper = Box::new(SpyWhisper::new("the budget is approved"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![
            ("<|zh|>", "我们的预算"),
            ("<|en|>", "sensevoice english"),
            ("<|zh|>", "好的"),
        ]));

        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);
        let out = router
            .transcribe(&audio(9.0), &[(0.0, 3.0), (3.0, 6.0), (6.0, 9.0)])
            .unwrap();

        let texts: Vec<&str> = out.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(
            texts,
            vec!["我们的预算", "the budget is approved", "好的"],
            "each segment should come from the model that suits its language"
        );
    }

    #[test]
    fn only_the_english_segments_are_sent_to_whisper() {
        // Proves the routing decision itself, not just the text: Whisper must
        // never be asked about the Chinese spans, or the cost argument for
        // doing this at all collapses.
        let (whisper, asked) = SpyWhisper::watched("english");
        let whisper = Box::new(whisper);
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![
            ("<|zh|>", "中文"),
            ("<|en|>", "ignored"),
            ("<|zh|>", "更多中文"),
        ]));

        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);
        router
            .transcribe(&audio(9.0), &[(0.0, 3.0), (3.0, 6.0), (6.0, 9.0)])
            .unwrap();

        let asked = asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![(3.0, 6.0)],
            "Whisper should see the English span and nothing else"
        );
    }

    #[test]
    fn with_no_sense_voice_downloaded_everything_goes_to_whisper() {
        // The English-only user's path. Must be indistinguishable from the
        // behaviour before routing existed.
        let whisper = Box::new(SpyWhisper::new("hello"));
        let router = RoutingTranscriber::new(whisper, None, SpeechEngine::Auto);

        let out = router
            .transcribe(&audio(6.0), &[(0.0, 3.0), (3.0, 6.0)])
            .unwrap();

        assert_eq!(out.len(), 2);
        assert!(!router.routes());
    }

    #[test]
    fn the_whisper_override_bypasses_routing_entirely() {
        let whisper = Box::new(SpyWhisper::new("whisper said this"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![("<|zh|>", "中文")]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Whisper);

        let out = router.transcribe(&audio(3.0), &[(0.0, 3.0)]).unwrap();

        assert_eq!(out[0].2, "whisper said this");
        assert!(!router.routes(), "forcing a model is not routing");
    }

    #[test]
    fn the_sense_voice_override_keeps_english_on_sense_voice() {
        // "Always SenseVoice" has to mean it, or the setting is a lie.
        let whisper = Box::new(SpyWhisper::new("whisper would say this"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![(
            "<|en|>",
            "sensevoice english",
        )]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::SenseVoice);

        let out = router.transcribe(&audio(3.0), &[(0.0, 3.0)]).unwrap();

        assert_eq!(out[0].2, "sensevoice english");
    }

    #[test]
    fn without_spans_whisper_handles_it_even_for_chinese() {
        // SenseVoice returns one untimed blob however much audio it is given.
        // For a whole recording that is a transcript with a single timestamp,
        // which is worse than a slightly weaker transcript that is navigable.
        let whisper = Box::new(SpyWhisper::new("segmented by whisper"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![(
            "<|zh|>",
            "一大段没有时间戳的文字",
        )]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);

        let out = router.transcribe(&audio(30.0), &[]).unwrap();

        assert_eq!(out[0].2, "segmented by whisper");
    }

    #[test]
    fn an_unknown_language_keeps_the_text_sense_voice_produced() {
        // Not English, so not re-run — guessing would replace a real transcript
        // with one from a model that was not chosen for that language.
        let whisper = Box::new(SpyWhisper::new("whisper guess"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![("", "text anyway")]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);

        let out = router.transcribe(&audio(3.0), &[(0.0, 3.0)]).unwrap();

        assert_eq!(out[0].2, "text anyway");
    }

    #[test]
    fn a_segment_neither_model_heard_anything_in_is_dropped_not_blanked() {
        let whisper = Box::new(SpyWhisper::new(""));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![("<|zh|>", "   ")]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);

        let out = router.transcribe(&audio(3.0), &[(0.0, 3.0)]).unwrap();

        assert!(out.is_empty(), "empty text should not become a blank line");
    }

    #[test]
    fn language_tags_are_read_with_or_without_sherpas_wrapper() {
        assert_eq!(normalize_lang("<|en|>"), "en");
        assert_eq!(normalize_lang("en"), "en");
        assert_eq!(normalize_lang(" <|ZH|> "), "zh");
        assert_eq!(normalize_lang(""), "");
    }

    #[test]
    fn a_span_past_the_end_of_the_audio_is_skipped_rather_than_panicking() {
        // Diarization spans come from a model. A span that runs past the buffer
        // must not index out of bounds.
        let whisper = Box::new(SpyWhisper::new("x"));
        let sense_voice = Box::new(ScriptedSenseVoice::new(vec![("<|zh|>", "中文")]));
        let router = RoutingTranscriber::new(whisper, Some(sense_voice), SpeechEngine::Auto);

        let out = router
            .transcribe(&audio(1.0), &[(0.0, 1.0), (5.0, 9.0)])
            .unwrap();

        assert_eq!(out.len(), 1);
    }
}
