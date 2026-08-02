//! Where audio comes from.
//!
//! [`AudioSource`] is the single seam between portable capture logic and the
//! platform. Everything above it — buffering, flushing, the state machine, the
//! disk guard — is tested here against [`FakeSource`]. Plan B2 adds two macOS
//! implementations (CoreAudio mic via `cpal`, system audio via
//! ScreenCaptureKit) and changes nothing else.
//!
//! The trait is **pull-based** on purpose. Both real macOS sources are
//! push-based (they hand you a callback), so each adapts by writing into a
//! ring buffer that `read` drains. Paying that small cost inside the platform
//! layer keeps the session loop, and every test of it, synchronous and
//! deterministic.

use anyhow::Result;

use super::SAMPLE_RATE;

/// A stream of mono `f32` samples at [`SAMPLE_RATE`], in `-1.0..=1.0`.
pub trait AudioSource: Send {
    /// Appends whatever audio is available right now to `out`. Appending
    /// nothing is normal and not an error — it means the source has produced
    /// no new audio since the last call.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<()>;

    /// True once the source will never produce audio again (device unplugged,
    /// screen sharing stopped, fixture exhausted). The session finalizes the
    /// recording rather than spinning forever.
    fn is_finished(&self) -> bool;

    /// Releases the underlying device. Called exactly once, on stop.
    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    /// A short name for error messages, e.g. `"microphone"`.
    fn label(&self) -> &str;
}

/// A scripted source for tests: hands out a fixed buffer in fixed-size chunks,
/// then reports finished. Optionally fails on a chosen chunk so the "device
/// died mid-recording" path can be exercised.
pub struct FakeSource {
    label: String,
    samples: Vec<f32>,
    chunk: usize,
    cursor: usize,
    /// Chunk index (0-based) at which `read` returns an error instead of audio.
    fail_at_chunk: Option<usize>,
    chunks_read: usize,
}

impl FakeSource {
    /// A source that yields `secs` seconds of a quiet sine wave in 0.1 s
    /// chunks — real signal, so level meters and sample counts mean something.
    pub fn tone(label: &str, secs: f64) -> Self {
        let total = (secs * SAMPLE_RATE as f64) as usize;
        let samples = (0..total)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.25
            })
            .collect();
        FakeSource {
            label: label.to_string(),
            samples,
            chunk: SAMPLE_RATE as usize / 10,
            cursor: 0,
            fail_at_chunk: None,
            chunks_read: 0,
        }
    }

    /// A source over exactly these samples, delivered in `chunk`-sized reads.
    pub fn from_samples(label: &str, samples: Vec<f32>, chunk: usize) -> Self {
        FakeSource {
            label: label.to_string(),
            samples,
            chunk: chunk.max(1),
            cursor: 0,
            fail_at_chunk: None,
            chunks_read: 0,
        }
    }

    /// Makes the `n`-th `read` (0-based) fail, simulating a device that drops
    /// out partway through a recording.
    pub fn failing_at_chunk(mut self, n: usize) -> Self {
        self.fail_at_chunk = Some(n);
        self
    }
}

impl AudioSource for FakeSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        if self.fail_at_chunk == Some(self.chunks_read) {
            self.chunks_read += 1;
            anyhow::bail!("{} disconnected", self.label);
        }
        self.chunks_read += 1;
        let end = (self.cursor + self.chunk).min(self.samples.len());
        out.extend_from_slice(&self.samples[self.cursor..end]);
        self.cursor = end;
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.cursor >= self.samples.len()
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// A source that never produces audio and is never finished — stands in for
/// the system-audio track on a platform that can't capture it (that is, here).
/// Meeting mode on Linux therefore records a mic track and an empty system
/// track rather than failing outright.
pub struct SilentSource {
    label: String,
}

impl SilentSource {
    pub fn new(label: &str) -> Self {
        SilentSource {
            label: label.to_string(),
        }
    }
}

impl AudioSource for SilentSource {
    fn read(&mut self, _out: &mut Vec<f32>) -> Result<()> {
        Ok(())
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn label(&self) -> &str {
        &self.label
    }
}
