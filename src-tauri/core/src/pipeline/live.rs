//! Chunked live transcription for the expanded overlay.
//!
//! This is intentionally batch-at-a-time. A short energy VAD holds back
//! silence, emits a partial chunk while a speaker is still talking, and emits
//! a final chunk after the hangover. Whisper runs over those chunks through
//! the same `Transcriber` trait used by the finished-recording pipeline; this
//! module does not create a second speech engine.

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};

use crate::capture::session::{CaptureTrack, LiveSample};
use crate::models::{ModelCache, ModelLease};

const SAMPLE_RATE: usize = 16_000;
const FRAME_SAMPLES: usize = 160;
const DEFAULT_THRESHOLD: f32 = 0.012;
const DEFAULT_PARTIAL_INTERVAL: usize = SAMPLE_RATE * 3 / 2;
const DEFAULT_MAX_CHUNK: usize = SAMPLE_RATE * 4;
const DEFAULT_HANGOVER: usize = SAMPLE_RATE / 2;
const DEFAULT_OVERLAP: usize = SAMPLE_RATE / 5;

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
            self.pending.extend_from_slice(frame);
            self.since_emit += frame.len();

            if rms(frame) >= self.threshold {
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
/// thread's first instruction until the capture sends `Finish`.
pub struct LiveTranscriptHandle {
    sender: mpsc::Sender<LiveSample>,
    events: Arc<Mutex<VecDeque<LiveTranscriptEvent>>>,
    join: Option<JoinHandle<()>>,
    finished: bool,
}

impl LiveTranscriptHandle {
    pub fn start(cache: Arc<ModelCache>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let events_for_thread = Arc::clone(&events);
        let join = thread::Builder::new()
            .name("notetaker-live-transcript".to_string())
            .spawn(move || run_worker(cache, receiver, events_for_thread))
            .expect("live transcript thread should start");
        Self {
            sender,
            events,
            join: Some(join),
            finished: false,
        }
    }

    pub fn sender(&self) -> mpsc::Sender<LiveSample> {
        self.sender.clone()
    }

    pub fn drain_events(&self) -> Vec<LiveTranscriptEvent> {
        let mut queue = self.events.lock().unwrap_or_else(|p| p.into_inner());
        queue.drain(..).collect()
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let _ = self.sender.send(LiveSample::Finish);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
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
) {
    let lease = match cache.acquire() {
        Ok(lease) => lease,
        Err(error) => {
            log::warn!("live transcript unavailable: {error:#}");
            return;
        }
    };
    let mut mic = TrackState::new("me");
    let mut system = TrackState::new("them");

    while let Ok(message) = receiver.recv() {
        match message {
            LiveSample::Samples(packet) => {
                let state = match packet.track {
                    CaptureTrack::Mic => &mut mic,
                    CaptureTrack::System => &mut system,
                };
                let chunks = state.chunker.push(&packet.samples);
                for chunk in chunks {
                    transcribe_chunk(&lease, state.speaker, &chunk, &events);
                }
            }
            LiveSample::Finish => {
                for state in [&mut mic, &mut system] {
                    if let Some(chunk) = state.chunker.finish() {
                        transcribe_chunk(&lease, state.speaker, &chunk, &events);
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
) {
    let result = lease.transcriber().transcribe(&chunk.samples, &[]);
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
    events
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push_back(LiveTranscriptEvent {
            speaker: speaker.to_string(),
            text,
            is_partial: !chunk.is_final,
            is_final: chunk.is_final,
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
