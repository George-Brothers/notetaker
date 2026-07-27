//! Incremental, crash-tolerant writing of one audio track.
//!
//! A lecture can run for two hours before anyone asks for it back, so the file
//! is kept valid on disk the whole time instead of only at the end: the WAV
//! header is rewritten every [`FLUSH_INTERVAL_SECS`] of audio, which is what
//! caps a power cut's cost at that much. The rest of this module exists to
//! feed the record bar — a running sample count for the timer, a peak level
//! for the meter — so the session never has to re-read the file it is writing.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};

use super::{FLUSH_INTERVAL_SECS, SAMPLE_RATE};

/// 16-bit PCM: what `pipeline::audio::load_mono_16k` reads and what every
/// downstream model wants, so incoming `f32` is scaled on the way in and
/// nothing resamples or reformats later.
const BITS_PER_SAMPLE: u16 = 16;

/// One track of a recording, written as it is captured.
pub struct TrackWriter {
    path: PathBuf,
    /// `None` once finalized. That is what makes [`TrackWriter::finalize`]
    /// idempotent and late writes harmless — the session can reach the same
    /// track from a user's Stop, the disk guard, and a dead source.
    writer: Option<WavWriter<BufWriter<File>>>,
    samples: u64,
    since_flush: u64,
    peak: f32,
}

impl TrackWriter {
    /// Creates `path` and writes its header immediately, so a valid (empty)
    /// wav exists before the first sample arrives rather than after the first
    /// flush.
    pub fn create(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let spec = WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: BITS_PER_SAMPLE,
            sample_format: SampleFormat::Int,
        };
        let writer = WavWriter::create(&path, spec)
            .with_context(|| format!("creating {}", path.display()))?;
        Ok(TrackWriter {
            path,
            writer: Some(writer),
            samples: 0,
            since_flush: 0,
            peak: 0.0,
        })
    }

    /// Appends `frames`, rewriting the header once more than
    /// [`FLUSH_INTERVAL_SECS`] of audio has piled up since the last one.
    ///
    /// Writing to a finalized track is a silent no-op, not an error: the disk
    /// guard can close a track while the pump still has frames in hand for it,
    /// and dropping those beats propagating an error at that point.
    pub fn write(&mut self, frames: &[f32]) -> Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };

        let mut peak = self.peak;
        for &frame in frames {
            // Saturate rather than let a misbehaving device's out-of-range
            // sample wrap into a loud click at the opposite polarity.
            let clamped = frame.clamp(-1.0, 1.0);
            peak = peak.max(clamped.abs());
            writer
                .write_sample((clamped * i16::MAX as f32).round() as i16)
                .with_context(|| format!("writing audio to {}", self.path.display()))?;
        }
        self.peak = peak;
        self.samples += frames.len() as u64;
        self.since_flush += frames.len() as u64;

        if self.since_flush >= FLUSH_INTERVAL_SECS * SAMPLE_RATE as u64 {
            writer
                .flush()
                .with_context(|| format!("flushing {}", self.path.display()))?;
            self.since_flush = 0;
        }
        Ok(())
    }

    /// The loudest sample since the previous call, then resets. The meter asks
    /// "how loud since I last looked", not "how loud has this recording ever
    /// been" — a single door slam should not peg the bar for an hour.
    pub fn take_peak(&mut self) -> f32 {
        std::mem::replace(&mut self.peak, 0.0)
    }

    /// Samples handed to this track, including any still sitting in the
    /// buffer.
    pub fn samples_written(&self) -> u64 {
        self.samples
    }

    /// Seconds of audio captured on this track.
    pub fn duration_s(&self) -> f64 {
        self.samples as f64 / SAMPLE_RATE as f64
    }

    /// Writes the final header and closes the file. Idempotent.
    pub fn finalize(&mut self) -> Result<()> {
        match self.writer.take() {
            Some(writer) => writer
                .finalize()
                .with_context(|| format!("finalizing {}", self.path.display())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::capture::{FLUSH_INTERVAL_SECS, SAMPLE_RATE};
    use crate::pipeline::audio::load_mono_16k;

    /// `secs` seconds of a 440 Hz tone at `amp`, the same shape a real mic
    /// hands over.
    fn tone(secs: f64, amp: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * amp
            })
            .collect()
    }

    fn on_disk(path: &Path) -> Vec<f32> {
        load_mono_16k(path).expect("track must be readable by the pipeline's loader")
    }

    #[test]
    fn written_audio_round_trips_through_the_pipeline_loader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio-mic.wav");

        let frames = tone(0.5, 0.5);
        let mut track = TrackWriter::create(&path).unwrap();
        track.write(&frames).unwrap();
        track.finalize().unwrap();

        let loaded = on_disk(&path);
        assert_eq!(loaded.len(), frames.len());
        for (want, got) in frames.iter().zip(&loaded) {
            assert!(
                (want - got).abs() < 1e-3,
                "sample drifted through the file: wanted {want}, got {got}"
            );
        }
        assert_eq!(track.duration_s(), 0.5);
        assert_eq!(track.samples_written(), frames.len() as u64);
    }

    #[test]
    fn a_crash_loses_at_most_one_flush_interval_of_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio-mic.wav");

        let mut track = TrackWriter::create(&path).unwrap();
        let chunk = vec![0.25f32; SAMPLE_RATE as usize / 10];
        let tenths = (FLUSH_INTERVAL_SECS * 2 + 2) * 10;
        for _ in 0..tenths {
            track.write(&chunk).unwrap();
        }

        // The power goes out here: nothing is finalized, we just read what a
        // recovery pass would find sitting on disk.
        let survived = on_disk(&path).len() as u64;
        let written = track.samples_written();
        assert!(survived > 0, "nothing reached disk at all");
        assert!(
            written - survived <= FLUSH_INTERVAL_SECS * SAMPLE_RATE as u64,
            "a crash would have lost {} samples, past the {FLUSH_INTERVAL_SECS}s budget",
            written - survived
        );
    }

    #[test]
    fn peak_reports_the_loudest_sample_since_the_last_poll() {
        let dir = tempfile::tempdir().unwrap();
        let mut track = TrackWriter::create(dir.path().join("audio-mic.wav")).unwrap();

        track.write(&[0.8, -0.2, 0.1]).unwrap();
        assert!((track.take_peak() - 0.8).abs() < 1e-6);

        track.write(&[0.05, -0.1]).unwrap();
        assert!(
            (track.take_peak() - 0.1).abs() < 1e-6,
            "the meter must not still be showing the earlier loud passage"
        );

        assert_eq!(
            track.take_peak(),
            0.0,
            "a poll with no audio since the last one reads silent"
        );
    }

    #[test]
    fn out_of_range_samples_are_clamped_rather_than_wrapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio-mic.wav");

        let mut track = TrackWriter::create(&path).unwrap();
        track.write(&[2.0, -2.0, 0.0]).unwrap();
        track.finalize().unwrap();

        let loaded = on_disk(&path);
        assert!(
            loaded[0] > 0.99 && loaded[1] < -0.99,
            "an over-range sample must saturate, not wrap into a click: {loaded:?}"
        );
    }

    #[test]
    fn finalize_is_idempotent_and_later_writes_cannot_corrupt_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio-mic.wav");

        let frames = tone(0.2, 0.3);
        let mut track = TrackWriter::create(&path).unwrap();
        track.write(&frames).unwrap();
        track.finalize().unwrap();
        track.finalize().unwrap();

        track.write(&frames).unwrap();
        assert_eq!(on_disk(&path).len(), frames.len());
        assert_eq!(track.samples_written(), frames.len() as u64);
    }
}
