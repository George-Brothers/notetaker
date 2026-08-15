//! The capture state machine behind Start / Pause / Resume / Stop.
//!
//! Two rules shape everything here. **The recording folder exists before the
//! first sample does**, so a crash mid-lecture leaves something the recovery
//! pass can repair rather than nothing at all. And **captured audio is never
//! thrown away to report a problem**: a dead microphone, a full disk, or a
//! file that will not close cleanly all end the recording, finalize what is
//! already on disk, and leave a plain-English sentence in `meta.capture_note`
//! — they never bubble up as an error that loses the recording id. That note
//! is a field of its own rather than `meta.error` because the queue clears
//! `error` on every retry, and no amount of reprocessing brings back audio the
//! disk never took.
//!
//! [`Session::pump`] is deliberately a single synchronous step rather than a
//! loop hidden inside a thread: the app owns the cadence, and every test here
//! drives capture one step at a time with no sleeping and no timing luck.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc::{SyncSender, TrySendError}, Arc};

use anyhow::{Context, Result};
use chrono::Local;

use crate::storage::{Mode, RecordingRef, Status, Store};

use super::source::AudioSource;
use super::track::TrackWriter;
use super::{
    CaptureLevels, CaptureState, CaptureStatus, DiskSpace, MIC_TRACK, MIN_FREE_MB, SYSTEM_TRACK,
};

/// One track's source paired with the file it feeds.
struct Channel {
    source: Box<dyn AudioSource>,
    writer: TrackWriter,
    /// Set once the source has failed or run dry. The file stays open until
    /// stop regardless, so whatever it captured is still finalized properly.
    done: bool,
}

/// Which read-only capture track a live consumer is observing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTrack {
    Mic,
    System,
}

/// A cloned sample packet for the live path. The recording writer never lends
/// out its buffer: the tee owns this copy and can process it independently.
#[derive(Debug)]
pub struct CapturedSamples {
    pub track: CaptureTrack,
    pub samples: Vec<f32>,
}

/// Message sent to a live consumer. `Finish` is available to graceful test or
/// future producers; the runtime's Stop path uses atomic cancellation instead
/// so it never waits for a model worker to flush.
pub enum LiveSample {
    Samples(CapturedSamples),
    Finish,
}

/// The capture-side end of the opt-in live pipeline. It is deliberately a
/// bounded, try-send-only queue: model inference may lag or be cancelled, but
/// microphone writes must never wait for it.
#[derive(Clone)]
pub struct LiveSampleSender {
    sender: SyncSender<LiveSample>,
    dropped_packets: Arc<AtomicU64>,
}

impl LiveSampleSender {
    pub(crate) fn new(
        sender: SyncSender<LiveSample>,
        dropped_packets: Arc<AtomicU64>,
    ) -> LiveSampleSender {
        LiveSampleSender {
            sender,
            dropped_packets,
        }
    }

    /// Attempts to hand one packet to live transcription without ever
    /// blocking the capture thread.
    pub fn try_send(&self, sample: LiveSample) -> bool {
        match self.sender.try_send(sample) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                let dropped = self.dropped_packets.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped == 1 || dropped.is_power_of_two() {
                    log::warn!("live transcription is behind; dropped {dropped} live packets");
                }
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn dropped_packets(&self) -> u64 {
        self.dropped_packets.load(Ordering::Relaxed)
    }
}

/// A live recording: one or two audio sources, their files, and the rules for
/// when to stop.
pub struct Session {
    /// The session's own handle on the store. `Store` is just a root path, so
    /// copying it costs a `PathBuf` and saves the app from lending a borrow
    /// into an object that outlives the call that made it.
    store: Store,
    rec: RecordingRef,
    disk: Box<dyn DiskSpace>,
    state: CaptureState,
    mic: Channel,
    /// `None` for in-person recordings, which have no system audio — and so
    /// no `audio-system.wav` on disk for the pipeline to trip over.
    system: Option<Channel>,
    /// Reused between pumps so an hour of capture doesn't churn allocations.
    buf: Vec<f32>,
    /// Plain-language reasons this recording ended early or lost a track,
    /// joined into `meta.capture_note` on stop.
    notes: Vec<String>,
    /// Whether the final `meta.json` write landed. Tracked apart from the
    /// state so a stop whose save failed can be retried without closing the
    /// tracks — or releasing the devices — a second time.
    saved: bool,
    /// Optional read-only tee. It is bounded and try-send-only; a slow
    /// recognizer can lose live-only packets but can never back-pressure the
    /// durable capture writer.
    live_tee: Option<LiveSampleSender>,
}

impl Session {
    /// Creates the recording folder and its `meta.json`, opens a file per
    /// track, and starts recording.
    ///
    /// Meeting mode requires `system_source`; in-person mode ignores it, since
    /// the pipeline treats an `audio-system.wav` as a promise that there is
    /// someone else on the call.
    pub fn start(
        store: &Store,
        mode: Mode,
        title: &str,
        mic_source: Box<dyn AudioSource>,
        system_source: Option<Box<dyn AudioSource>>,
        disk: Box<dyn DiskSpace>,
    ) -> Result<Session> {
        Self::start_with_tee(
            store,
            mode,
            title,
            mic_source,
            system_source,
            disk,
            None,
        )
    }

    /// Starts a session with an optional read-only sample tee.
    pub fn start_with_tee(
        store: &Store,
        mode: Mode,
        title: &str,
        mic_source: Box<dyn AudioSource>,
        system_source: Option<Box<dyn AudioSource>>,
        disk: Box<dyn DiskSpace>,
        live_tee: Option<LiveSampleSender>,
    ) -> Result<Session> {
        // Validate before creating anything, so a bad call leaves no empty
        // folder behind for the recovery sweep to puzzle over.
        let system_source = match mode {
            Mode::Meeting => Some(system_source.context(
                "meeting recordings capture system audio, but no system audio source was supplied",
            )?),
            Mode::InPerson => None,
        };

        let rec = store.create_recording(title, mode, Local::now())?;

        let mic = Channel {
            writer: TrackWriter::create(rec.dir.join(format!("{MIC_TRACK}.wav")))?,
            source: mic_source,
            done: false,
        };
        let system = match system_source {
            Some(source) => Some(Channel {
                writer: TrackWriter::create(rec.dir.join(format!("{SYSTEM_TRACK}.wav")))?,
                source,
                done: false,
            }),
            None => None,
        };

        Ok(Session {
            store: Store::new(&store.root),
            rec,
            disk,
            state: CaptureState::Recording,
            mic,
            system,
            buf: Vec::new(),
            notes: Vec::new(),
            saved: false,
            live_tee,
        })
    }

    /// One step of the capture loop: check the disk, then move whatever audio
    /// is available from each source into its file.
    ///
    /// Returns the state the session is in afterwards. [`CaptureState::Idle`]
    /// means it stopped itself — the disk ran low, or every source is gone —
    /// and the recording is already finalized and saved.
    pub fn pump(&mut self) -> Result<CaptureState> {
        if self.state == CaptureState::Idle {
            return Ok(CaptureState::Idle);
        }

        // Checked while paused too: the volume can fill from anywhere, and a
        // paused session is still holding files open on it.
        if let Some(reason) = self.disk_trouble() {
            self.notes.push(reason);
            self.stop()?;
            return Ok(CaptureState::Idle);
        }

        let capturing = self.state == CaptureState::Recording;
        let mut buf = std::mem::take(&mut self.buf);
        drain(
            &mut self.mic,
            CaptureTrack::Mic,
            capturing,
            &mut buf,
            &mut self.notes,
            self.live_tee.as_ref(),
        );
        if let Some(system) = self.system.as_mut() {
            drain(
                system,
                CaptureTrack::System,
                capturing,
                &mut buf,
                &mut self.notes,
                self.live_tee.as_ref(),
            );
        }
        self.buf = buf;

        let all_gone = self.mic.done && self.system.as_ref().is_none_or(|s| s.done);
        if capturing && all_gone {
            self.stop()?;
            return Ok(CaptureState::Idle);
        }
        Ok(self.state)
    }

    /// Stops consuming audio without closing anything. The files stay open, so
    /// resuming continues the same track instead of opening a second one and
    /// the paused stretch simply never appears in the recording.
    pub fn pause(&mut self) {
        if self.state == CaptureState::Recording {
            self.state = CaptureState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == CaptureState::Paused {
            self.state = CaptureState::Recording;
        }
    }

    /// Finalizes every track, writes the real duration and `Recorded` status,
    /// and returns the recording id. Idempotent — the disk guard, a dead
    /// source, and the user's Stop button all arrive here.
    ///
    /// The only failure it reports is a `meta.json` that would not write, and
    /// calling `stop` again retries exactly that: the audio is already closed
    /// on disk by then, so the recording is never lost to a failed save.
    pub fn stop(&mut self) -> Result<String> {
        if self.state != CaptureState::Idle {
            self.state = CaptureState::Idle;

            close(&mut self.mic, &mut self.notes);
            if let Some(system) = self.system.as_mut() {
                close(system, &mut self.notes);
            }

            self.rec.meta.duration_s = self.elapsed_s();
            self.rec.meta.status = Status::Recorded;
            if !self.notes.is_empty() {
                self.rec.meta.capture_note = Some(self.notes.join(" "));
            }
        }

        if !self.saved {
            self.store.save_meta(&self.rec)?;
            self.saved = true;
        }
        Ok(self.rec.meta.id.clone())
    }

    /// A snapshot for the record bar. Takes `&mut self` because the level
    /// meters report the peak *since the last poll*, and reading them clears
    /// them.
    pub fn status(&mut self) -> CaptureStatus {
        let levels = self.levels();
        CaptureStatus {
            state: self.state,
            // `None` once the session has stopped: the snapshot answers "what
            // is recording right now", and by then nothing is.
            // `Finishing` is the runtime's word for the stretch after the
            // session object is gone, so a live session never reports it.
            mode: match self.state {
                CaptureState::Idle | CaptureState::Finishing => None,
                CaptureState::Recording | CaptureState::Paused => Some(self.rec.meta.mode),
            },
            recording_id: Some(self.rec.meta.id.clone()),
            elapsed_s: self.elapsed_s(),
            mic_level: levels.mic_level,
            system_level: levels.system_level,
            disk_free_mb: self.disk.free_mb().unwrap_or(0),
        }
    }

    /// The fast-moving part of [`CaptureStatus`]. Reading a level consumes its
    /// short peak window, so a one-off loud noise cannot pin the meter.
    pub fn levels(&mut self) -> CaptureLevels {
        CaptureLevels {
            mic_level: self.mic.writer.take_peak(),
            system_level: self.system.as_mut().map_or(0.0, |s| s.writer.take_peak()),
        }
    }

    /// Seconds of audio captured, paused time excluded — paused audio is never
    /// written, so the longer track's length *is* the elapsed time. The longer
    /// of the two, because one track can die while the other keeps going.
    pub fn elapsed_s(&self) -> f64 {
        let system = self.system.as_ref().map_or(0.0, |s| s.writer.duration_s());
        self.mic.writer.duration_s().max(system)
    }

    pub fn state(&self) -> CaptureState {
        self.state
    }

    /// The recording being captured, for the caller that needs its folder or
    /// id once capture is over.
    pub fn recording(&self) -> &RecordingRef {
        &self.rec
    }

    /// Why capture must stop right now, if the volume can no longer be trusted
    /// with more audio. Worded for someone who wants their notes back, not for
    /// a developer reading a log: an unreadable volume counts as "no space",
    /// because refusing to record is recoverable and losing a lecture is not.
    fn disk_trouble(&self) -> Option<String> {
        match self.disk.free_mb() {
            Some(mb) if mb >= MIN_FREE_MB => None,
            Some(mb) => Some(format!(
                "Recording stopped because this computer is nearly out of storage space \
                 (about {mb} MB left). Everything recorded up to that point was saved. \
                 Free up some space, then start a new recording."
            )),
            None => Some(
                "Recording stopped because Notetaker could not read how much storage space \
                 is left, and it will not risk filling up the disk. Everything recorded up \
                 to that point was saved."
                    .to_string(),
            ),
        }
    }
}

/// Moves whatever one source has ready into its file. A source that errors, or
/// a file that will not take the audio, is marked done and left alone: its
/// track is still finalized on stop, because audio already captured is never
/// worth throwing away over the track that follows it.
fn drain(
    ch: &mut Channel,
    track: CaptureTrack,
    capturing: bool,
    buf: &mut Vec<f32>,
    notes: &mut Vec<String>,
    live_tee: Option<&LiveSampleSender>,
) {
    if ch.done {
        return;
    }

    buf.clear();
    if let Err(e) = ch.source.read(buf) {
        log::warn!("{} read failed: {e:#}", ch.source.label());
        notes.push(format!(
            "The {} stopped working partway through, so the recording ended there. \
             Everything captured before that was saved.",
            ch.source.label()
        ));
        ch.done = true;
        return;
    }

    // While paused the audio is read and dropped: real sources buffer, and a
    // paused session that stopped reading would hand back the paused audio on
    // resume as if it had been recorded.
    if capturing && !buf.is_empty() {
        if let Some(tee) = live_tee {
            // `try_send` cannot wait for model inference, and the writer still
            // receives the original `buf` unchanged below.
            let _ = tee.try_send(LiveSample::Samples(CapturedSamples {
                track,
                samples: buf.clone(),
            }));
        }
        if let Err(e) = ch.writer.write(buf) {
            log::warn!("{} write failed: {e:#}", ch.source.label());
            notes.push(format!(
                "Notetaker could not save the {} to disk, so the recording ended there. \
                 Everything saved before that point is still there.",
                ch.source.label()
            ));
            ch.done = true;
            return;
        }
    }

    if ch.source.is_finished() {
        ch.done = true;
    }
}

/// Releases the device and closes the file. Neither failure is propagated: by
/// now the audio is on disk, and the caller needs the recording id far more
/// than it needs an error. A file that would not close cleanly is called out
/// in `meta.capture_note` so the recovery pass and the user both know to look.
fn close(ch: &mut Channel, notes: &mut Vec<String>) {
    if let Err(e) = ch.source.stop() {
        log::warn!("releasing {} failed: {e:#}", ch.source.label());
    }
    if let Err(e) = ch.writer.finalize() {
        log::warn!("finalizing {} failed: {e:#}", ch.source.label());
        notes.push(format!(
            "The {} recording may be cut short — Notetaker could not close its file cleanly.",
            ch.source.label()
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use super::*;
    use crate::capture::source::{FakeSource, SilentSource};
    use crate::capture::{FixedDisk, MIN_FREE_MB, SAMPLE_RATE};
    use crate::pipeline::audio::load_mono_16k;
    use crate::storage::{Mode, Status, Store};

    const CHUNK: usize = SAMPLE_RATE as usize / 10;

    fn healthy_disk() -> Box<FixedDisk> {
        Box::new(FixedDisk(Some(MIN_FREE_MB * 20)))
    }

    /// Runs the loop the way the app's capture thread will, with a hard bound
    /// so a bug shows up as a failure rather than a hung test.
    fn pump_until_idle(session: &mut Session) {
        for _ in 0..1000 {
            if session.pump().unwrap() == CaptureState::Idle {
                return;
            }
        }
        panic!("session never stopped on its own");
    }

    fn track(dir: &Path, stem: &str) -> Vec<f32> {
        load_mono_16k(&dir.join(format!("{stem}.wav")))
            .expect("track must be readable by the pipeline's loader")
    }

    fn wav_count(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .count()
    }

    /// Reads the recording back the way the app would after a restart, so
    /// assertions are about what is on disk rather than what is in memory.
    fn meta_on_disk(store: &Store, id: &str) -> crate::storage::Meta {
        store
            .scan()
            .unwrap()
            .into_iter()
            .find(|r| r.meta.id == id)
            .expect("recording must be on disk")
            .meta
    }

    #[test]
    fn start_leaves_a_recoverable_directory_before_any_audio_is_pumped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 1.0)),
            None,
            healthy_disk(),
        )
        .unwrap();

        let rec_dir = session.recording().dir.clone();
        assert!(
            rec_dir.join("meta.json").exists(),
            "a crash one second in must still leave a readable recording folder"
        );
        assert!(rec_dir.join("audio-mic.wav").exists());
        assert_eq!(store.scan().unwrap().len(), 1);
        assert_eq!(session.state(), CaptureState::Recording);
    }

    #[test]
    fn in_person_capture_writes_one_track_matching_what_the_source_produced() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 2.0)),
            // Even handed a system source, in-person mode must ignore it.
            Some(Box::new(SilentSource::new("system audio"))),
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        assert!(
            !rec_dir.join("audio-system.wav").exists(),
            "in-person recordings have no system audio to capture"
        );
        assert_eq!(wav_count(&rec_dir), 1);
        assert_eq!(track(&rec_dir, "audio-mic").len(), 2 * SAMPLE_RATE as usize);

        let meta = meta_on_disk(&store, &id);
        assert_eq!(meta.status, Status::Recorded);
        assert!(
            meta.capture_note.is_none(),
            "clean capture must not leave a note"
        );
        assert!(meta.error.is_none(), "clean capture must not set an error");
    }

    #[test]
    fn read_only_live_tee_keeps_pipeline_wav_byte_identical() {
        fn run(dir: &Path, tee: bool) -> (Vec<u8>, Vec<f32>) {
            let store = Store::new(dir);
            let samples: Vec<f32> = (0..(SAMPLE_RATE as usize / 2))
                .map(|i| ((i % 97) as f32 - 48.0) / 100.0)
                .collect();
            let (tx, rx) = std::sync::mpsc::sync_channel(256);
            let live_tee = tee.then(|| {
                LiveSampleSender::new(
                    tx,
                    std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                )
            });
            let mut session = Session::start_with_tee(
                &store,
                Mode::InPerson,
                "Contract",
                Box::new(FakeSource::from_samples("microphone", samples, 137)),
                None,
                healthy_disk(),
                live_tee,
            )
            .unwrap();
            let path = session.recording().dir.join("audio-mic.wav");
            pump_until_idle(&mut session);
            session.stop().unwrap();
            drop(session);

            let observed = rx
                .into_iter()
                .filter_map(|message| match message {
                    LiveSample::Samples(packet) => Some(packet.samples),
                    LiveSample::Finish => None,
                })
                .flatten()
                .collect();
            (std::fs::read(path).unwrap(), observed)
        }

        let without_dir = tempfile::tempdir().unwrap();
        let with_dir = tempfile::tempdir().unwrap();
        let (without, without_observed) = run(without_dir.path(), false);
        let (with, with_observed) = run(with_dir.path(), true);
        assert!(without_observed.is_empty());
        assert!(!with_observed.is_empty());
        assert_eq!(without, with, "the live path changed pipeline output bytes");
    }

    #[test]
    fn meeting_mode_captures_both_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::Meeting,
            "Standup",
            Box::new(FakeSource::tone("microphone", 1.0)),
            Some(Box::new(FakeSource::tone("system audio", 1.0))),
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        session.stop().unwrap();

        assert_eq!(wav_count(&rec_dir), 2);
        assert_eq!(track(&rec_dir, "audio-mic").len(), SAMPLE_RATE as usize);
        assert_eq!(track(&rec_dir, "audio-system").len(), SAMPLE_RATE as usize);
    }

    #[test]
    fn meeting_mode_refuses_to_start_without_a_system_source() {
        // Better to fail loudly than to record a meeting the pipeline will
        // reject hours later for a missing system track.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let err = match Session::start(
            &store,
            Mode::Meeting,
            "Standup",
            Box::new(FakeSource::tone("microphone", 1.0)),
            None,
            healthy_disk(),
        ) {
            Ok(_) => panic!("a meeting with no system source must not start"),
            Err(e) => e,
        };

        assert!(format!("{err:#}").contains("system audio"), "{err:#}");
        assert!(
            store.scan().unwrap().is_empty(),
            "a refused start must not leave an empty recording folder behind"
        );
    }

    #[test]
    fn pause_and_resume_produce_one_continuous_file_without_the_paused_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        // Each 0.1 s chunk carries a distinct constant, so the file itself
        // shows which stretches were recorded and which were dropped.
        let level = |chunk: usize| (chunk + 1) as f32 / 20.0;
        let samples: Vec<f32> = (0..10).flat_map(|c| vec![level(c); CHUNK]).collect();

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::from_samples("microphone", samples, CHUNK)),
            None,
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();

        for _ in 0..3 {
            session.pump().unwrap();
        }
        session.pause();
        assert_eq!(session.state(), CaptureState::Paused);
        for _ in 0..3 {
            session.pump().unwrap();
        }
        session.resume();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        assert_eq!(
            wav_count(&rec_dir),
            1,
            "resume must continue the same file, not open a second one"
        );

        let audio = track(&rec_dir, "audio-mic");
        assert_eq!(audio.len(), 7 * CHUNK, "paused audio must not be written");
        assert!((audio[0] - level(0)).abs() < 1e-3);
        assert!(
            (audio[3 * CHUNK] - level(6)).abs() < 1e-3,
            "the resumed audio must butt straight up against the paused point"
        );
        for dropped in 3..6 {
            assert!(
                audio.iter().all(|s| (s - level(dropped)).abs() > 1e-3),
                "audio from the paused stretch leaked into the file"
            );
        }

        assert!(
            (session.elapsed_s() - 0.7).abs() < 1e-6,
            "elapsed counts captured audio only"
        );
        assert!((meta_on_disk(&store, &id).duration_s - 0.7).abs() < 1e-6);
    }

    #[test]
    fn duration_after_stop_is_the_real_captured_length() {
        // Carry-over M5: duration_s used to stay 0.0 forever, so every
        // recording showed as empty in the list.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 1.5)),
            None,
            healthy_disk(),
        )
        .unwrap();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        let meta = meta_on_disk(&store, &id);
        assert!(
            (meta.duration_s - 1.5).abs() < 1e-6,
            "got {}",
            meta.duration_s
        );
    }

    #[test]
    fn stopping_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 0.5)),
            None,
            healthy_disk(),
        )
        .unwrap();
        pump_until_idle(&mut session);
        let first = session.stop().unwrap();
        let second = session.stop().unwrap();

        assert_eq!(first, second);
        assert!((meta_on_disk(&store, &first).duration_s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_stop_whose_meta_write_fails_can_be_retried_without_losing_the_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 0.5)),
            None,
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        for _ in 0..3 {
            session.pump().unwrap();
        }

        // Make `meta.json` unwritable the crude, portable way.
        let meta_path = rec_dir.join("meta.json");
        std::fs::remove_file(&meta_path).unwrap();
        std::fs::create_dir(&meta_path).unwrap();
        assert!(session.stop().is_err(), "a failed save must be reported");

        std::fs::remove_dir(&meta_path).unwrap();
        let id = session.stop().unwrap();

        assert_eq!(track(&rec_dir, "audio-mic").len(), 3 * CHUNK);
        let meta = meta_on_disk(&store, &id);
        assert_eq!(meta.status, Status::Recorded);
        assert!(
            (meta.duration_s - 0.3).abs() < 1e-6,
            "the retry must still write the real duration, got {}",
            meta.duration_s
        );
    }

    #[test]
    fn low_free_space_stops_the_session_with_an_actionable_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 5.0)),
            None,
            Box::new(FixedDisk(Some(10))),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        let meta = meta_on_disk(&store, &id);
        assert_eq!(meta.status, Status::Recorded);
        assert_eq!(
            meta.error, None,
            "a capture problem is not a processing failure, so it must not sit \
             in the field the queue clears on retry"
        );
        let message = meta
            .capture_note
            .expect("the user must be told why it stopped");
        assert!(message.contains("storage space"), "{message}");
        assert!(
            !message.contains("MIN_FREE_MB") && !message.contains("Err"),
            "the message must read as plain English, not a log line: {message}"
        );
        // Whatever was captured is still a file the pipeline can open.
        assert!(track(&rec_dir, "audio-mic").is_empty());
    }

    #[test]
    fn unreadable_free_space_stops_the_session_rather_than_gambling() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 5.0)),
            None,
            Box::new(FixedDisk(None)),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        let message = meta_on_disk(&store, &id)
            .capture_note
            .expect("the user must be told why it stopped");
        assert!(message.contains("storage space"), "{message}");
        assert!(track(&rec_dir, "audio-mic").is_empty());
    }

    /// Free space that falls off a cliff partway through — the "disk filled up
    /// mid-lecture" case a fixed reading cannot express.
    struct DrainingDisk {
        polls: Cell<u32>,
        healthy_polls: u32,
    }

    impl DiskSpace for DrainingDisk {
        fn free_mb(&self) -> Option<u64> {
            let n = self.polls.get();
            self.polls.set(n + 1);
            if n < self.healthy_polls {
                Some(MIN_FREE_MB * 20)
            } else {
                Some(10)
            }
        }
    }

    #[test]
    fn a_disk_that_fills_mid_recording_keeps_everything_captured_so_far() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 5.0)),
            None,
            Box::new(DrainingDisk {
                polls: Cell::new(0),
                healthy_polls: 3,
            }),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        assert_eq!(
            track(&rec_dir, "audio-mic").len(),
            3 * CHUNK,
            "audio captured before the disk filled must survive"
        );
        let meta = meta_on_disk(&store, &id);
        assert!((meta.duration_s - 0.3).abs() < 1e-6);
        assert!(meta.capture_note.unwrap().contains("storage space"));
    }

    #[test]
    fn a_source_that_dies_mid_stream_still_finalizes_a_playable_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 5.0).failing_at_chunk(5)),
            None,
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        let id = session.stop().unwrap();

        assert_eq!(
            track(&rec_dir, "audio-mic").len(),
            5 * CHUNK,
            "the half second captured before the mic died must not be lost"
        );
        let meta = meta_on_disk(&store, &id);
        assert_eq!(meta.status, Status::Recorded);
        assert!((meta.duration_s - 0.5).abs() < 1e-6);
        let message = meta
            .capture_note
            .expect("the user must be told the mic dropped out");
        assert!(message.contains("microphone"), "{message}");
    }

    #[test]
    fn a_dead_system_track_does_not_take_the_mic_down_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::Meeting,
            "Standup",
            Box::new(FakeSource::tone("microphone", 1.0)),
            Some(Box::new(
                FakeSource::tone("system audio", 1.0).failing_at_chunk(2),
            )),
            healthy_disk(),
        )
        .unwrap();
        let rec_dir = session.recording().dir.clone();
        pump_until_idle(&mut session);
        session.stop().unwrap();

        assert_eq!(
            track(&rec_dir, "audio-mic").len(),
            SAMPLE_RATE as usize,
            "the mic must keep recording after the system track drops out"
        );
        assert_eq!(track(&rec_dir, "audio-system").len(), 2 * CHUNK);
    }

    #[test]
    fn status_feeds_the_record_bar_with_levels_that_reset_each_poll() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let loud: Vec<f32> = vec![0.75; CHUNK * 2];
        let mut session = Session::start(
            &store,
            Mode::Meeting,
            "Standup",
            Box::new(FakeSource::from_samples("microphone", loud, CHUNK)),
            Some(Box::new(SilentSource::new("system audio"))),
            healthy_disk(),
        )
        .unwrap();

        session.pump().unwrap();
        let id = session.recording().meta.id.clone();
        let status = session.status();
        assert_eq!(status.state, CaptureState::Recording);
        assert_eq!(status.recording_id.as_deref(), Some(id.as_str()));
        assert!((status.elapsed_s - 0.1).abs() < 1e-6);
        assert!(
            (status.mic_level - 0.75).abs() < 1e-3,
            "{}",
            status.mic_level
        );
        assert_eq!(status.system_level, 0.0, "a silent system track reads flat");
        assert_eq!(status.disk_free_mb, MIN_FREE_MB * 20);

        assert_eq!(
            session.status().mic_level,
            0.0,
            "the meter must fall back to silent when no new audio arrived"
        );
    }

    #[test]
    fn status_reports_which_mode_is_recording_and_nothing_once_stopped() {
        // Without this, only the component that pressed Record knows what kind
        // of recording is running — a menu bar or a recovered session polling
        // the same snapshot cannot tell a meeting from an in-person lecture.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::Meeting,
            "Standup",
            Box::new(FakeSource::tone("microphone", 0.3)),
            Some(Box::new(SilentSource::new("system audio"))),
            healthy_disk(),
        )
        .unwrap();

        session.pump().unwrap();
        assert_eq!(session.status().mode, Some(Mode::Meeting));

        session.pause();
        assert_eq!(
            session.status().mode,
            Some(Mode::Meeting),
            "a paused recording is still a meeting"
        );
        session.resume();

        session.stop().unwrap();
        assert_eq!(
            session.status().mode,
            None,
            "nothing is being recorded once the session has stopped"
        );
        assert_eq!(CaptureStatus::idle(1234).mode, None);
    }

    #[test]
    fn status_reports_in_person_mode_for_an_in_person_recording() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let mut session = Session::start(
            &store,
            Mode::InPerson,
            "Lecture 3",
            Box::new(FakeSource::tone("microphone", 0.3)),
            None,
            healthy_disk(),
        )
        .unwrap();

        session.pump().unwrap();
        assert_eq!(session.status().mode, Some(Mode::InPerson));
    }
}
