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
use chrono::Utc;

use crate::pipeline::audio::load_mono_16k;
use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
use crate::pipeline::llm::LlmClient;
use crate::pipeline::merge::{label_speakers, merge_meeting, to_transcript_md};
use crate::pipeline::suggest::{suggest_task, suggest_title, Suggestion};
use crate::pipeline::summarize::summarize;
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
    let stage_count = match rec.meta.mode {
        Mode::InPerson => 6,
        Mode::Meeting => 8,
    };

    let utterances = match rec.meta.mode {
        Mode::InPerson => {
            let mic =
                require_track(&rec.dir, "audio-mic").context("locating in-person mic track")?;
            mark_stage(store, rec, "Loading audio", 1, stage_count)?;
            let samples = timed(&mut stages, "load-audio", || load_mono_16k(&mic))?;

            mark_stage(store, rec, "Separating speakers", 2, stage_count)?;
            let spans = timed(&mut stages, "diarize", || deps.diarizer.diarize(&samples))
                .context("diarization")?;
            let ranges = spans_to_ranges(&spans);
            mark_stage(store, rec, "Transcribing", 3, stage_count)?;
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

            mark_stage(store, rec, "Loading system audio", 1, stage_count)?;
            let system_samples = timed(&mut stages, "load-system", || load_mono_16k(&system))?;
            mark_stage(store, rec, "Loading microphone audio", 2, stage_count)?;
            let mic_samples = timed(&mut stages, "load-mic", || load_mono_16k(&mic))?;

            mark_stage(store, rec, "Separating speakers", 3, stage_count)?;
            let spans = timed(&mut stages, "diarize", || {
                deps.diarizer.diarize(&system_samples)
            })
            .context("diarization")?;
            let ranges = spans_to_ranges(&spans);
            mark_stage(store, rec, "Transcribing call audio", 4, stage_count)?;
            let others_texts = timed(&mut stages, "transcribe-system", || {
                deps.transcriber.transcribe(&system_samples, &ranges)
            })
            .context("transcription (system track)")?;
            mark_stage(store, rec, "Transcribing microphone audio", 5, stage_count)?;
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
    // `summarize`'s module note.
    let notes_md = crate::notes::read(&rec.dir);

    mark_stage(
        store,
        rec,
        "Writing AI notes",
        if rec.meta.mode == Mode::InPerson {
            4
        } else {
            6
        },
        stage_count,
    )?;
    let summary_md = timed(&mut stages, "summarize", || {
        summarize(
            deps.llm,
            &transcript_md,
            &notes_md,
            rec.meta.template.as_deref(),
        )
    })
    .context("summarization")?;
    mark_stage(
        store,
        rec,
        "Choosing a folder",
        if rec.meta.mode == Mode::InPerson {
            5
        } else {
            7
        },
        stage_count,
    )?;
    let suggestion = timed(&mut stages, "suggest-task", || {
        suggest_task(deps.llm, &summary_md, &deps.tasks)
    })
    .context("task suggestion")?;

    // A better title than the timestamp, offered for one-click accept. Never
    // fails the run: the transcript and summary are already good, and a
    // recording with a dull title is a far better outcome than a lost one.
    mark_stage(
        store,
        rec,
        "Finishing up",
        if rec.meta.mode == Mode::InPerson {
            6
        } else {
            8
        },
        stage_count,
    )?;
    let suggested_title = timed(&mut stages, "suggest-title", || {
        Ok(suggest_title(deps.llm, &summary_md).unwrap_or_else(|e| {
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
    // Reload because the stage markers above are intentionally durable while
    // this run is in flight; a stale clone would erase the latest marker just
    // before the queue flips the recording to Ready.
    let mut updated = store.reload(rec)?;
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
            rec.meta.queue_control = crate::storage::QueueControl::Active;
            rec.meta.processing_started_at = None;
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

/// Persists a human-readable stage before the expensive work begins. The
/// timestamp is written only once per run, so the UI can calculate elapsed time
/// without another timer or a made-up completion percentage.
fn mark_stage(store: &Store, rec: &RecordingRef, stage: &str, index: u8, count: u8) -> Result<()> {
    let mut updated = store.reload(rec)?;
    updated.meta.processing_stage = Some(stage.to_string());
    updated.meta.processing_stage_index = index;
    updated.meta.processing_stage_count = count;
    if updated.meta.processing_started_at.is_none() {
        updated.meta.processing_started_at = Some(Utc::now().to_rfc3339());
    }
    if let Some(started) = updated
        .meta
        .processing_started_at
        .as_deref()
        .and_then(|value| value.parse::<chrono::DateTime<chrono::FixedOffset>>().ok())
    {
        updated.meta.processing_elapsed_s = Utc::now()
            .signed_duration_since(started.with_timezone(&Utc))
            .num_milliseconds()
            .max(0) as f64
            / 1000.0;
    }
    store.save_meta(&updated)
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
        let deps = PipelineDeps {
            transcriber: &transcriber,
            diarizer: &diarizer,
            llm: &llm,
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
        let deps = PipelineDeps {
            transcriber: &NeverTranscriber,
            diarizer: &NeverDiarizer,
            llm: &llm,
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
