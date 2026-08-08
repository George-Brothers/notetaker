//! Pipeline orchestrator: takes one recorded meeting or class and produces
//! its finished notes — transcript, summary, and a task suggestion — writing
//! them next to the audio.
//!
//! This is straight-line glue over the stage modules. It never changes a
//! recording's [`Status`](crate::storage::Status): the queue owns that, so a
//! crash here leaves the recording `Processing` for the startup sweep
//! ([`requeue_stale`]) to recover.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};

use crate::pipeline::audio::load_mono_16k;
use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
use crate::pipeline::llm::{KeepAlive, LlmClient};
use crate::pipeline::merge::{label_speakers, merge_meeting, to_transcript_md};
use crate::pipeline::suggest::{suggest_task_with_keep_alive, suggest_title_with_keep_alive, Suggestion};
use crate::pipeline::summarize::summarize_with_keep_alive;
use crate::pipeline::transcribe::Transcriber;
use crate::pipeline::Utterance;
use crate::storage::{Mode, RecordingRef, StageTiming, Status, Store};

const SUGGESTED_TASK_FILE: &str = "suggested_task.txt";

/// Sidecar holding a better title than the timestamp the recording was created
/// with, awaiting a one-click accept. A sidecar rather than a `Meta` field for
/// the same reason as the task suggestion: it is a transient offer, not durable
/// metadata, and accepting or ignoring it should not rewrite `meta.json`.
const SUGGESTED_TITLE_FILE: &str = "suggested_title.txt";

/// The models and knowledge one processing run needs. Borrowed, because a
/// single loaded model serves every recording in the queue.
pub struct PipelineDeps<'a> {
    pub transcriber: &'a dyn Transcriber,
    pub diarizer: &'a dyn Diarizer,
    pub llm: &'a LlmClient,
    /// The persisted summary shapes available when this recording is run.
    pub templates: &'a [crate::templates::Template],
    /// The user's free-form instruction for every meeting summary.
    pub summary_prompt: &'a str,
    /// Existing task names, for the "which task does this belong to" step.
    pub tasks: Vec<String>,
}

/// What one run produced, also written to disk inside the recording folder.
pub struct ProcessOutput {
    pub transcript_md: String,
    pub summary_md: String,
    pub suggestion: Suggestion,
}

/// Processes one recording end to end, writing `transcript.md`, `summary.md`,
/// and the `suggested_task.txt` sidecar into `rec.dir`, and recording
/// `meta.speakers` plus per-stage timings into `meta.json`. Does not touch
/// `meta.status`.
///
/// Meeting mode transcribes the owner's mic track wholesale as `George`
/// (that track is physically his, so it is never diarized) and interleaves it
/// with the diarized system track. In-person mode diarizes the single mic
/// track and labels everyone `Speaker N`.
pub fn process_recording(
    store: &Store,
    deps: &PipelineDeps,
    rec: &RecordingRef,
) -> Result<ProcessOutput> {
    let mut stages: Vec<StageTiming> = Vec::new();
    let mut speakers: BTreeMap<String, String> = BTreeMap::new();

    let utterances = match rec.meta.mode {
        Mode::InPerson => {
            let mic =
                require_track(&rec.dir, "audio-mic").context("locating in-person mic track")?;
            let samples = timed(&mut stages, "load-audio", || load_mono_16k(&mic))?;

            let spans = timed(&mut stages, "diarize", || deps.diarizer.diarize(&samples))
                .context("diarization")?;
            let ranges = spans_to_ranges(&spans);
            let texts = timed(&mut stages, "transcribe", || {
                deps.transcriber.transcribe(&samples, &ranges)
            })
            .context("transcription")?;

            record_diarized_speakers(&mut speakers, &spans);
            label_speakers(&spans, &texts)
        }
        Mode::Meeting => {
            let system = require_track(&rec.dir, "audio-system")
                .context("meeting recordings need a system-audio track; none found")?;
            let mic = require_track(&rec.dir, "audio-mic").context("locating meeting mic track")?;

            let system_samples = timed(&mut stages, "load-system", || load_mono_16k(&system))?;
            let mic_samples = timed(&mut stages, "load-mic", || load_mono_16k(&mic))?;

            let spans = timed(&mut stages, "diarize", || {
                deps.diarizer.diarize(&system_samples)
            })
            .context("diarization")?;
            let ranges = spans_to_ranges(&spans);
            // Nobody spoke on the system track (nothing was playing, or what
            // played had no voices). Skip its transcription entirely: the
            // Transcriber contract reads an empty range list as "transcribe
            // the whole file", and Whisper run over minutes of silence
            // hallucinates markers into the transcript.
            let others_texts = if spans.is_empty() {
                timed(&mut stages, "transcribe-system", || Ok(Vec::new()))?
            } else {
                timed(&mut stages, "transcribe-system", || {
                    deps.transcriber.transcribe(&system_samples, &ranges)
                })
                .context("transcription (system track)")?
            };
            let mic_texts = timed(&mut stages, "transcribe-mic", || {
                deps.transcriber.transcribe(&mic_samples, &[])
            })
            .context("transcription (mic track)")?;

            record_diarized_speakers(&mut speakers, &spans);
            speakers.insert("george".to_string(), "George".to_string());

            let others = label_speakers(&spans, &others_texts);
            let george: Vec<Utterance> = mic_texts
                .into_iter()
                .map(|(start_s, end_s, text)| Utterance {
                    start_s,
                    end_s,
                    speaker: "George".to_string(),
                    text,
                })
                .collect();
            merge_meeting(george, others)
        }
    };

    let transcript_md = to_transcript_md(&rec.meta.title, &utterances);

    // Whatever the user typed during the recording. The summarizer is asked to
    // expand on it rather than summarize the call from scratch — see
    // `summarize`'s module note. Starred moments ride along as an addendum:
    // a star pressed mid-meeting means "this part mattered", which is exactly
    // the signal the summarizer should weight.
    let mut notes_md = crate::notes::read(&rec.dir);
    let highlights = crate::notes::read_highlights(&rec.dir);
    if !highlights.trim().is_empty() {
        if !notes_md.is_empty() {
            notes_md.push_str("\n\n");
        }
        notes_md.push_str("Moments I starred as important during the recording:\n");
        notes_md.push_str(&highlights);
    }

    let summary_md = timed(&mut stages, "summarize", || {
        summarize_with_keep_alive(
            deps.llm,
            &transcript_md,
            &notes_md,
            rec.meta.template.as_deref(),
            deps.templates,
            deps.summary_prompt,
            KeepAlive::Batch,
        )
    })
    .context("summarization")?;
    let suggestion = timed(&mut stages, "suggest-task", || {
        suggest_task_with_keep_alive(deps.llm, &summary_md, &deps.tasks, KeepAlive::Batch)
    })
    .context("task suggestion")?;

    // A better title than the timestamp, offered for one-click accept. Never
    // fails the run: the transcript and summary are already good, and a
    // recording with a dull title is a far better outcome than a lost one.
    let suggested_title = timed(&mut stages, "suggest-title", || {
        Ok(suggest_title_with_keep_alive(deps.llm, &summary_md, KeepAlive::Final)
            .unwrap_or_else(|e| {
            log::warn!("could not suggest a title for {}: {e:#}", rec.meta.id);
            None
        }))
    })?;

    // Persist outputs next to the audio.
    write_file(&rec.dir.join("transcript.md"), &transcript_md)?;
    write_file(&rec.dir.join("summary.md"), &summary_md)?;
    write_suggestion(&rec.dir, &suggestion)?;
    write_suggested_title(&rec.dir, suggested_title.as_deref())?;

    // Record speakers and stage timings without touching status. Both are
    // real `Meta` fields now, so a plain save_meta persists them durably.
    let mut updated = rec.clone();
    updated.meta.speakers = speakers;
    updated.meta.stages = stages;
    store.save_meta(&updated)?;

    Ok(ProcessOutput {
        transcript_md,
        summary_md,
        suggestion,
    })
}

/// Recovers from a crash during processing: any recording left `Processing`
/// on disk (because the app died mid-run) is returned to `Queued` so it will
/// be retried. Returns how many were requeued.
pub fn requeue_stale(store: &Store) -> Result<usize> {
    let mut n = 0;
    for mut rec in store.scan()? {
        if rec.meta.status == Status::Processing {
            rec.meta.status = Status::Queued;
            store.save_meta(&rec)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Finds `<prefix>.wav` or `<prefix>.flac` in `dir`, erroring if neither
/// exists — a missing expected track is a hard failure, never a silently
/// one-sided transcript.
fn require_track(dir: &Path, prefix: &str) -> Result<PathBuf> {
    for ext in ["flac", "wav"] {
        let p = dir.join(format!("{prefix}.{ext}"));
        if p.exists() {
            return Ok(p);
        }
    }
    bail!("no {prefix}.flac or {prefix}.wav in {}", dir.display())
}

fn spans_to_ranges(spans: &[SpeakerSpan]) -> Vec<(f32, f32)> {
    spans.iter().map(|s| (s.start_s, s.end_s)).collect()
}

/// Maps 0-based diarizer ids to the `"spk{n+1}" -> "Speaker {n+1}"`
/// convention that `merge::label_speakers` writes into the transcript and
/// that `api::rename_speaker` expects.
fn record_diarized_speakers(speakers: &mut BTreeMap<String, String>, spans: &[SpeakerSpan]) {
    for s in spans {
        let n = s.speaker + 1;
        speakers.insert(format!("spk{n}"), format!("Speaker {n}"));
    }
}

fn write_suggestion(dir: &Path, suggestion: &Suggestion) -> Result<()> {
    let body = suggestion.task.as_deref().unwrap_or("");
    write_file(&dir.join(SUGGESTED_TASK_FILE), body)
}

/// Writes the suggested title, or removes a stale one when this run produced
/// none. Removing matters on a reprocess: a title suggested from an earlier,
/// worse summary must not keep being offered after a better run declined to
/// suggest anything.
fn write_suggested_title(dir: &Path, title: Option<&str>) -> Result<()> {
    let path = dir.join(SUGGESTED_TITLE_FILE);
    match title {
        Some(t) => write_file(&path, t),
        None => {
            let _ = std::fs::remove_file(&path);
            Ok(())
        }
    }
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

/// Runs `f`, appending its wall-clock milliseconds to `stages` under `name`.
fn timed<T, F: FnOnce() -> Result<T>>(
    stages: &mut Vec<StageTiming>,
    name: &str,
    f: F,
) -> Result<T> {
    let start = Instant::now();
    let out = f();
    stages.push(StageTiming {
        stage: name.to_string(),
        ms: start.elapsed().as_millis() as u64,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::diarize::SherpaDiarizer;
    use crate::pipeline::transcribe::WhisperTranscriber;
    use crate::storage::Mode;
    use chrono::TimeZone;

    fn repo_path(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn models_present() -> bool {
        repo_path("../../models/ggml-tiny.bin").exists()
            && repo_path("../../models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx").exists()
            && repo_path("../../models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx")
                .exists()
    }

    /// The release gate: a real recording through the real speech + speaker
    /// models produces a transcript with both languages, a summary, and an
    /// honored task suggestion. Ollama is the only mocked part.
    #[test]
    fn golden_bilingual_pipeline_produces_transcript_summary_and_suggestion() {
        if !models_present() {
            eprintln!("SKIP: run scripts/fetch-whisper-model.sh and fetch-diarization-models.sh");
            return;
        }

        // Ollama stand-in. Both LLM calls (summarize, then suggest) hit the
        // same mock; it returns the suggestion JSON, which summarize stores as
        // the summary text and suggest_task parses — so we assert the summary
        // is non-empty rather than checking for a specific "TL;DR" heading.
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant",
                    "content": "{\"task\": \"Accounting 302\", \"confidence\": 0.9}" }
            }));
        });

        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let rec = store
            .create_recording("Lecture 3", Mode::InPerson, created)
            .unwrap();
        std::fs::copy(
            repo_path("../../fixtures/bilingual.wav"),
            rec.dir.join("audio-mic.wav"),
        )
        .unwrap();

        let transcriber =
            WhisperTranscriber::load(&repo_path("../../models/ggml-tiny.bin")).unwrap();
        let diarizer = SherpaDiarizer::load(
            &repo_path("../../models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
            &repo_path("../../models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx"),
        )
        .unwrap();
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "test".to_string(),
        };
        let templates = crate::templates::defaults();
        let deps = PipelineDeps {
            transcriber: &transcriber,
            diarizer: &diarizer,
            llm: &llm,
            templates: &templates,
            summary_prompt: "",
            tasks: vec!["Accounting 302".to_string()],
        };

        let out = process_recording(&store, &deps, &rec).unwrap();

        // Transcript written, with speaker labels and both scripts present.
        let transcript = std::fs::read_to_string(rec.dir.join("transcript.md")).unwrap();
        assert!(
            transcript.contains("**"),
            "expected speaker labels: {transcript}"
        );
        assert!(
            transcript
                .chars()
                .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "expected Chinese characters: {transcript}"
        );
        assert!(
            transcript.chars().any(|c| c.is_ascii_alphabetic()),
            "expected English text: {transcript}"
        );

        // Summary and suggestion written.
        assert!(rec.dir.join("summary.md").exists());
        assert!(!out.summary_md.is_empty());
        assert_eq!(out.suggestion.task.as_deref(), Some("Accounting 302"));
        let sidecar = std::fs::read_to_string(rec.dir.join("suggested_task.txt")).unwrap();
        assert_eq!(sidecar, "Accounting 302");

        // Stage timings recorded in meta.json.
        let meta_raw = std::fs::read_to_string(rec.dir.join("meta.json")).unwrap();
        assert!(
            meta_raw.contains("stages"),
            "stage timings missing: {meta_raw}"
        );
    }

    /// Write a real, decodable 16 kHz mono WAV of `n` samples so the loader
    /// accepts it; the content never matters because the fakes below never
    /// look at it beyond its length.
    fn write_test_wav(path: &Path, n: usize) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..n {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// A meeting whose system track holds no speech (nothing was playing
    /// through the speakers) must still yield a transcript from the mic
    /// track. Found on real hardware 2026-08-05: sherpa reports "no
    /// speakers" as an error, and the whole recording failed — discarding a
    /// perfectly good mic track. The diarizer fake returns the zero spans
    /// that case now produces; the transcriber fake fails the test if the
    /// pipeline tries to transcribe the speakerless system track wholesale.
    #[test]
    fn meeting_with_speakerless_system_track_keeps_the_mic_transcript() {
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/api/chat");
            then.status(200).json_body(serde_json::json!({
                "message": { "role": "assistant",
                    "content": "{\"task\": \"Accounting 302\", \"confidence\": 0.9}" }
            }));
        });

        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 5, 21, 48, 0)
            .unwrap();
        let rec = store
            .create_recording("Meeting", Mode::Meeting, created)
            .unwrap();
        // Mic and system tracks distinguishable by length alone.
        write_test_wav(&rec.dir.join("audio-mic.wav"), 3200);
        write_test_wav(&rec.dir.join("audio-system.wav"), 1600);

        struct NoSpeakers;
        impl Diarizer for NoSpeakers {
            fn diarize(&self, _: &[f32]) -> Result<Vec<SpeakerSpan>> {
                Ok(Vec::new())
            }
        }
        struct MicOnly {
            calls: std::sync::atomic::AtomicU32,
        }
        impl Transcriber for MicOnly {
            fn transcribe(
                &self,
                samples: &[f32],
                spans: &[(f32, f32)],
            ) -> Result<Vec<(f32, f32, String)>> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert!(spans.is_empty(), "the mic track is transcribed wholesale");
                assert_eq!(
                    samples.len(),
                    3200,
                    "only the mic track may be transcribed; a speakerless \
                     system track must be skipped, not sent wholesale"
                );
                Ok(vec![(0.0, 0.2, "the words that must survive".to_string())])
            }
        }

        let transcriber = MicOnly {
            calls: std::sync::atomic::AtomicU32::new(0),
        };
        let diarizer = NoSpeakers;
        let llm = LlmClient {
            base_url: server.base_url(),
            model: "test".to_string(),
        };
        let templates = crate::templates::defaults();
        let deps = PipelineDeps {
            transcriber: &transcriber,
            diarizer: &diarizer,
            llm: &llm,
            templates: &templates,
            summary_prompt: "",
            tasks: vec!["Accounting 302".to_string()],
        };

        let out = process_recording(&store, &deps, &rec).unwrap();

        assert_eq!(
            transcriber.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one transcription: the mic"
        );
        assert!(
            out.transcript_md.contains("George"),
            "the mic speaker survives: {}",
            out.transcript_md
        );
        assert!(
            out.transcript_md.contains("the words that must survive"),
            "the mic words survive: {}",
            out.transcript_md
        );
    }

    #[test]
    fn requeue_stale_resets_processing_to_queued_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();

        let mut stuck = store
            .create_recording("Stuck", Mode::InPerson, created)
            .unwrap();
        stuck.meta.status = Status::Processing;
        store.save_meta(&stuck).unwrap();

        let mut ready = store
            .create_recording("Done", Mode::InPerson, created)
            .unwrap();
        ready.meta.status = Status::Ready;
        store.save_meta(&ready).unwrap();

        assert_eq!(requeue_stale(&store).unwrap(), 1);

        let statuses: BTreeMap<String, Status> = store
            .scan()
            .unwrap()
            .into_iter()
            .map(|r| (r.meta.title.clone(), r.meta.status))
            .collect();
        assert_eq!(statuses["Stuck"], Status::Queued);
        assert_eq!(statuses["Done"], Status::Ready);
    }

    #[test]
    fn meeting_mode_missing_system_track_errors_with_stage_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let created = chrono::Local
            .with_ymd_and_hms(2026, 8, 4, 10, 2, 0)
            .unwrap();
        let rec = store
            .create_recording("Call", Mode::Meeting, created)
            .unwrap();
        // Only a mic track; the system track is missing.
        std::fs::write(rec.dir.join("audio-mic.wav"), b"not real audio").unwrap();

        struct NeverTranscriber;
        impl Transcriber for NeverTranscriber {
            fn transcribe(&self, _: &[f32], _: &[(f32, f32)]) -> Result<Vec<(f32, f32, String)>> {
                panic!("should not transcribe when a track is missing")
            }
        }
        struct NeverDiarizer;
        impl Diarizer for NeverDiarizer {
            fn diarize(&self, _: &[f32]) -> Result<Vec<SpeakerSpan>> {
                panic!("should not diarize when a track is missing")
            }
        }
        let llm = LlmClient {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "x".to_string(),
        };
        let templates = crate::templates::defaults();
        let deps = PipelineDeps {
            transcriber: &NeverTranscriber,
            diarizer: &NeverDiarizer,
            llm: &llm,
            templates: &templates,
            summary_prompt: "",
            tasks: vec![],
        };

        let err = match process_recording(&store, &deps, &rec) {
            Ok(_) => panic!("expected an error for the missing system track"),
            Err(e) => e,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("system-audio"),
            "error should name the missing system track: {msg}"
        );
    }
}
