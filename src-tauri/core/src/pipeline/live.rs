//! Chunked live transcription for the expanded overlay.
//!
//! This is intentionally batch-at-a-time. A short energy VAD holds back
//! silence, emits a partial chunk while a speaker is still talking, and emits
//! a final chunk after the hangover. Whisper runs over those chunks through
//! the same `Transcriber` trait used by the finished-recording pipeline; this
//! module does not create a second speech engine.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capture::session::{CaptureTrack, LiveSample, LiveSampleSender};
use crate::models::{ModelCache, ModelLease};

const SAMPLE_RATE: usize = 16_000;
const FRAME_SAMPLES: usize = 160;
const DEFAULT_THRESHOLD: f32 = 0.012;
const DEFAULT_PARTIAL_INTERVAL: usize = SAMPLE_RATE * 3 / 2;
const DEFAULT_MAX_CHUNK: usize = SAMPLE_RATE * 4;
const DEFAULT_HANGOVER: usize = SAMPLE_RATE / 2;
const DEFAULT_OVERLAP: usize = SAMPLE_RATE / 5;
const LIVE_QUEUE_CAPACITY: usize = 8;
const LIVE_EVENT_CAPACITY: usize = 256;

/// One event the overlay receives. The frontend owns presentation; this type
/// deliberately carries no timestamp or message id because the partial-merge
/// contract is speaker-scoped and ordered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTranscriptEvent {
    pub speaker: String,
    pub text: String,
    pub is_partial: bool,
    pub is_final: bool,
}

/// Counters for the opt-in live path. Dropped packets are live-only; the
/// durable WAV never uses this queue and therefore never loses audio here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTranscriptStats {
    pub dropped_packets: u64,
    pub dropped_events: u64,
    pub queue_capacity: usize,
}

impl Default for LiveTranscriptStats {
    fn default() -> Self {
        Self {
            dropped_packets: 0,
            dropped_events: 0,
            queue_capacity: LIVE_QUEUE_CAPACITY,
        }
    }
}

/// One VAD-chosen window sent to Whisper.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub is_final: bool,
}

/// Small, deterministic VAD chunker. It is deliberately independent of a
/// model file so the live path can start on a machine that has recording
/// enabled but is still downloading speech models. Production model access is
/// still gated by the ModelCache lease; the chunk boundary itself is cheap.
pub struct VadChunker {
    pending: Vec<f32>,
    speech_seen: bool,
    trailing_silence: usize,
    since_emit: usize,
    threshold: f32,
    partial_interval: usize,
    max_chunk: usize,
    hangover: usize,
    overlap: usize,
}

impl Default for VadChunker {
    fn default() -> Self {
        Self::new(
            DEFAULT_THRESHOLD,
            DEFAULT_PARTIAL_INTERVAL,
            DEFAULT_MAX_CHUNK,
            DEFAULT_HANGOVER,
            DEFAULT_OVERLAP,
        )
    }
}

impl VadChunker {
    pub fn new(
        threshold: f32,
        partial_interval: usize,
        max_chunk: usize,
        hangover: usize,
        overlap: usize,
    ) -> Self {
        Self {
            pending: Vec::new(),
            speech_seen: false,
            trailing_silence: 0,
            since_emit: 0,
            threshold,
            partial_interval: partial_interval.max(FRAME_SAMPLES),
            max_chunk: max_chunk.max(FRAME_SAMPLES),
            hangover: hangover.max(FRAME_SAMPLES),
            overlap: overlap.min(max_chunk.saturating_sub(FRAME_SAMPLES)),
        }
    }

    /// Adds samples and returns zero or more chunks. The returned partial
    /// chunks are snapshots; the VAD keeps its own buffer for the final pass.
    pub fn push(&mut self, samples: &[f32]) -> Vec<AudioChunk> {
        let mut output = Vec::new();
        for frame in samples.chunks(FRAME_SAMPLES) {
            // There is no useful transcript in leading silence. Dropping it
            // also keeps a muted microphone from growing `pending` forever
            // before the first utterance arrives.
            let frame_is_speech = rms(frame) >= self.threshold;
            if !self.speech_seen && !frame_is_speech {
                self.pending.clear();
                self.since_emit = 0;
                continue;
            }
            self.pending.extend_from_slice(frame);
            self.since_emit += frame.len();

            if frame_is_speech {
                self.speech_seen = true;
                self.trailing_silence = 0;
            } else if self.speech_seen {
                self.trailing_silence += frame.len();
            }

            if self.speech_seen && self.since_emit >= self.partial_interval {
                output.push(self.partial_snapshot());
                self.since_emit = 0;
            }

            if self.speech_seen && self.trailing_silence >= self.hangover {
                output.push(self.final_snapshot());
                self.reset();
            } else if self.speech_seen && self.pending.len() >= self.max_chunk {
                output.push(self.partial_snapshot());
                self.since_emit = 0;
            }
        }
        output
    }

    /// Flushes a last utterance when the capture session stops.
    pub fn finish(&mut self) -> Option<AudioChunk> {
        if self.speech_seen && !self.pending.is_empty() {
            Some(self.final_snapshot())
        } else {
            self.reset();
            None
        }
    }

    fn partial_snapshot(&mut self) -> AudioChunk {
        let split = self.pending.len().saturating_sub(self.overlap);
        let samples = self.pending.clone();
        if self.pending.len() >= self.max_chunk {
            self.pending = self.pending[split..].to_vec();
            self.trailing_silence = 0;
        }
        AudioChunk {
            samples,
            is_final: false,
        }
    }

    fn final_snapshot(&mut self) -> AudioChunk {
        let chunk = AudioChunk {
            samples: self.pending.clone(),
            is_final: true,
        };
        self.reset();
        chunk
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.speech_seen = false;
        self.trailing_silence = 0;
        self.since_emit = 0;
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The stateful partial-merge pattern used by the overlay. A new partial for a
/// speaker updates that speaker's last unfrozen line; a final event freezes it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartialTranscript {
    messages: Vec<LiveTranscriptEvent>,
}

impl PartialTranscript {
    pub fn apply(&mut self, event: LiveTranscriptEvent) {
        let existing = self.messages.iter().rposition(|message| {
            message.speaker == event.speaker && message.is_partial && !message.is_final
        });

        match existing {
            Some(index) => {
                self.messages[index] = LiveTranscriptEvent {
                    speaker: event.speaker,
                    text: event.text,
                    is_partial: event.is_partial,
                    is_final: event.is_final,
                };
            }
            None => self.messages.push(event),
        }
    }

    pub fn messages(&self) -> &[LiveTranscriptEvent] {
        &self.messages
    }
}

struct TrackState {
    speaker: &'static str,
    chunker: VadChunker,
}

impl TrackState {
    fn new(speaker: &'static str) -> Self {
        Self {
            speaker,
            chunker: VadChunker::default(),
        }
    }
}

/// A live worker fed by the capture tee. It owns the model lease from its
/// thread's first instruction until the capture is cancelled or an optional
/// graceful `Finish` message is received.
pub struct LiveTranscriptHandle {
    sender: Option<LiveSampleSender>,
    events: Arc<Mutex<VecDeque<LiveTranscriptEvent>>>,
    dropped_packets: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    finished: bool,
}

impl LiveTranscriptHandle {
    pub fn start(cache: Arc<ModelCache>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(LIVE_QUEUE_CAPACITY);
        let dropped_packets = Arc::new(AtomicU64::new(0));
        let sender = LiveSampleSender::new(sender, Arc::clone(&dropped_packets));
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let events_for_thread = Arc::clone(&events);
        let dropped_events = Arc::new(AtomicU64::new(0));
        let dropped_events_for_thread = Arc::clone(&dropped_events);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = Arc::clone(&cancel);
        let join = thread::Builder::new()
            .name("notetaker-live-transcript".to_string())
            .spawn(move || {
                run_worker(
                    cache,
                    receiver,
                    events_for_thread,
                    dropped_events_for_thread,
                    cancel_for_thread,
                )
            })
            .expect("live transcript thread should start");
        Self {
            sender: Some(sender),
            events,
            dropped_packets,
            dropped_events,
            cancel,
            join: Some(join),
            finished: false,
        }
    }

    pub fn sender(&self) -> Option<LiveSampleSender> {
        self.sender.clone()
    }

    pub fn drain_events(&self) -> Vec<LiveTranscriptEvent> {
        let mut queue = self.events.lock().unwrap_or_else(|p| p.into_inner());
        queue.drain(..).collect()
    }

    pub fn stats(&self) -> LiveTranscriptStats {
        LiveTranscriptStats {
            dropped_packets: self.dropped_packets.load(Ordering::Relaxed),
            dropped_events: self.dropped_events.load(Ordering::Relaxed),
            queue_capacity: LIVE_QUEUE_CAPACITY,
        }
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.cancel.store(true, Ordering::Release);
        // Dropping this last producer closes the bounded receiver once the
        // capture-side clone is dropped. The JoinHandle is intentionally
        // detached: Stop must not wait for model teardown.
        self.sender.take();
        self.join.take();
    }
}

impl Drop for LiveTranscriptHandle {
    fn drop(&mut self) {
        self.finish();
    }
}

fn run_worker(
    cache: Arc<ModelCache>,
    receiver: mpsc::Receiver<LiveSample>,
    events: Arc<Mutex<VecDeque<LiveTranscriptEvent>>>,
    dropped_events: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
) {
    if cancel.load(Ordering::Acquire) {
        return;
    }
    let lease = match cache.acquire() {
        Ok(lease) => lease,
        Err(error) => {
            log::warn!("live transcript unavailable: {error:#}");
            return;
        }
    };
    let mut mic = TrackState::new("me");
    let mut system = TrackState::new("them");

    while !cancel.load(Ordering::Acquire) {
        let message = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match message {
            LiveSample::Samples(packet) => {
                let state = match packet.track {
                    CaptureTrack::Mic => &mut mic,
                    CaptureTrack::System => &mut system,
                };
                let chunks = state.chunker.push(&packet.samples);
                for chunk in chunks.into_iter().filter(|chunk| chunk.is_final) {
                    transcribe_chunk(
                        &lease,
                        state.speaker,
                        &chunk,
                        &events,
                        &dropped_events,
                        &cancel,
                    );
                }
            }
            LiveSample::Finish => {
                if !cancel.load(Ordering::Acquire) {
                    for state in [&mut mic, &mut system] {
                        if let Some(chunk) = state.chunker.finish() {
                            transcribe_chunk(
                                &lease,
                                state.speaker,
                                &chunk,
                                &events,
                                &dropped_events,
                                &cancel,
                            );
                        }
                    }
                }
                break;
            }
        }
    }
}

fn transcribe_chunk(
    lease: &ModelLease,
    speaker: &str,
    chunk: &AudioChunk,
    events: &Arc<Mutex<VecDeque<LiveTranscriptEvent>>>,
    dropped_events: &Arc<AtomicU64>,
    cancel: &Arc<AtomicBool>,
) {
    if cancel.load(Ordering::Acquire) {
        return;
    }
    let result = lease.transcriber().transcribe(&chunk.samples, &[]);
    if cancel.load(Ordering::Acquire) {
        return;
    }
    let text = match result {
        Ok(spans) => spans
            .into_iter()
            .map(|(_, _, text)| text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Err(error) => {
            log::warn!("live {speaker} transcription failed: {error:#}");
            return;
        }
    };
    if text.trim().is_empty() {
        return;
    }
    let mut queue = events.lock().unwrap_or_else(|p| p.into_inner());
    if queue.len() >= LIVE_EVENT_CAPACITY {
        queue.pop_front();
        dropped_events.fetch_add(1, Ordering::Relaxed);
    }
    queue.push_back(LiveTranscriptEvent {
        speaker: speaker.to_string(),
        text,
        // The worker intentionally transcribes finalized VAD utterances only;
        // partial snapshots never enter this queue.
        is_partial: false,
        is_final: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ModelIdleUnload, SpeechEngine};
    use crate::models::cache::{LoadedModels, ModelPaths};
    use crate::pipeline::diarize::{Diarizer, SpeakerSpan};
    use crate::pipeline::transcribe::Transcriber;

    fn tone(value: f32, count: usize) -> Vec<f32> {
        vec![value; count]
    }

    #[test]
    fn chunker_holds_silence_and_emits_partial_then_final() {
        let mut chunker = VadChunker::new(0.01, 320, 2_000, 320, 160);
        assert!(chunker.push(&tone(0.0, 640)).is_empty());
        let partials = chunker.push(&tone(0.2, 640));
        assert!(partials.iter().any(|chunk| !chunk.is_final));
        let finals = chunker.push(&tone(0.0, 480));
        assert!(finals.iter().any(|chunk| chunk.is_final));
    }

    #[test]
    fn chunker_flushes_a_short_utterance_without_waiting_for_hangover() {
        let mut chunker = VadChunker::default();
        assert!(chunker.push(&tone(0.2, 640)).is_empty());
        let chunk = chunker.finish().expect("speech must be flushed");
        assert!(chunk.is_final);
        assert!(!chunk.samples.is_empty());
    }

    #[test]
    fn leading_silence_does_not_accumulate_without_bound() {
        let mut chunker = VadChunker::default();
        let silence = vec![0.0; SAMPLE_RATE * 10];

        assert!(chunker.push(&silence).is_empty());
        assert!(chunker.pending.is_empty());
        assert!(!chunker.speech_seen);
    }

    #[test]
    fn partials_mutate_in_place_and_final_freezes_the_line() {
        let mut transcript = PartialTranscript::default();
        transcript.apply(LiveTranscriptEvent {
            speaker: "me".into(),
            text: "we need".into(),
            is_partial: true,
            is_final: false,
        });
        transcript.apply(LiveTranscriptEvent {
            speaker: "me".into(),
            text: "we need a date".into(),
            is_partial: true,
            is_final: false,
        });
        assert_eq!(transcript.messages().len(), 1);
        transcript.apply(LiveTranscriptEvent {
            speaker: "me".into(),
            text: "we need a date".into(),
            is_partial: false,
            is_final: true,
        });
        transcript.apply(LiveTranscriptEvent {
            speaker: "me".into(),
            text: "and an owner".into(),
            is_partial: true,
            is_final: false,
        });
        assert_eq!(transcript.messages().len(), 2);
        assert!(transcript.messages()[0].is_final);
    }

    #[test]
    fn partials_for_two_speakers_keep_two_live_lines() {
        let mut transcript = PartialTranscript::default();
        for (speaker, text) in [("me", "hello"), ("them", "hi"), ("me", "hello there")] {
            transcript.apply(LiveTranscriptEvent {
                speaker: speaker.into(),
                text: text.into(),
                is_partial: true,
                is_final: false,
            });
        }
        assert_eq!(transcript.messages().len(), 2);
        assert_eq!(transcript.messages()[0].text, "hello there");
        assert_eq!(transcript.messages()[1].text, "hi");
    }

    #[test]
    fn live_sender_drops_when_full_without_blocking() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let sender = LiveSampleSender::new(
            tx,
            Arc::new(AtomicU64::new(0)),
        );
        let packet = || {
            LiveSample::Samples(crate::capture::session::CapturedSamples {
                track: CaptureTrack::Mic,
                samples: vec![0.1; FRAME_SAMPLES],
            })
        };
        assert!(sender.try_send(packet()));
        assert!(!sender.try_send(packet()));
        assert_eq!(sender.dropped_packets(), 1);
    }

    struct EmptyTranscriber;
    impl Transcriber for EmptyTranscriber {
        fn transcribe(&self, _: &[f32], _: &[(f32, f32)]) -> anyhow::Result<Vec<(f32, f32, String)>> {
            Ok(Vec::new())
        }
    }

    struct EmptyDiarizer;
    impl Diarizer for EmptyDiarizer {
        fn diarize(&self, _: &[f32]) -> anyhow::Result<Vec<SpeakerSpan>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn finish_requests_cancellation_without_joining_a_slow_model_worker() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let cache = Arc::new(crate::models::ModelCache::new(
            ModelPaths {
                speech: std::path::PathBuf::new(),
                segmentation: std::path::PathBuf::new(),
                embedding: std::path::PathBuf::new(),
                sense_voice: None,
                speech_engine: SpeechEngine::Whisper,
            },
            ModelIdleUnload::Never,
            move |_| {
                started_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .recv()
                    .unwrap();
                Ok(LoadedModels {
                    transcriber: Box::new(EmptyTranscriber),
                    diarizer: Box::new(EmptyDiarizer),
                })
            },
            None,
        ));
        let mut handle = LiveTranscriptHandle::start(cache);
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("the worker should enter model loading");

        let started = std::time::Instant::now();
        handle.finish();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "Stop must not synchronously wait for model teardown"
        );

        release_tx.send(()).unwrap();
    }
}
