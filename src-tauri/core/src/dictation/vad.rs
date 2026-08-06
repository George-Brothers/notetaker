//! Silero VAD integration and the small amount of temporal smoothing around it.
//!
//! Sherpa owns the model-level speech decision. `VadSmoother` only prevents a
//! single noisy frame from opening the meter and keeps it alive over short
//! inter-word gaps; it is pure so its safety properties can be unit-tested
//! without loading the ONNX artifact.

use std::path::Path;

use anyhow::Result;

const SAMPLE_RATE: usize = 16_000;

/// Onset/hangover smoothing expressed in samples at the fixed capture rate.
#[derive(Debug, Clone)]
pub struct VadSmoother {
    onset_samples: usize,
    hangover_samples: usize,
    detected_samples: usize,
    silent_samples: usize,
    active: bool,
}

impl VadSmoother {
    pub fn new(onset: std::time::Duration, hangover: std::time::Duration) -> Self {
        Self {
            onset_samples: duration_samples(onset),
            hangover_samples: duration_samples(hangover),
            detected_samples: 0,
            silent_samples: 0,
            active: false,
        }
    }

    /// Adds one detector decision and returns whether the meter/audio gate is
    /// open after smoothing it.
    pub fn push(&mut self, detected: bool, samples: usize) -> bool {
        if detected {
            self.detected_samples = self.detected_samples.saturating_add(samples);
            self.silent_samples = 0;
            if self.detected_samples >= self.onset_samples {
                self.active = true;
            }
        } else if self.active {
            self.silent_samples = self.silent_samples.saturating_add(samples);
            if self.silent_samples >= self.hangover_samples {
                self.active = false;
                self.detected_samples = 0;
                self.silent_samples = 0;
            }
        } else {
            self.detected_samples = 0;
        }
        self.active
    }

    pub fn active(&self) -> bool {
        self.active
    }
}

fn duration_samples(duration: std::time::Duration) -> usize {
    (duration.as_secs_f64() * SAMPLE_RATE as f64).round() as usize
}

/// The runtime gate around sherpa's Silero detector.
pub struct SileroGate {
    vad: sherpa_rs::silero_vad::SileroVad,
    smoother: VadSmoother,
    voiced: Vec<f32>,
}

impl SileroGate {
    pub fn open(model: &Path) -> Result<Self> {
        if !model.is_file() {
            anyhow::bail!("Silero VAD model is missing at {}", model.display());
        }
        let config = sherpa_rs::silero_vad::SileroVadConfig {
            model: model.to_string_lossy().into_owned(),
            min_silence_duration: 0.35,
            min_speech_duration: 0.10,
            max_speech_duration: 30.0,
            threshold: 0.35,
            sample_rate: SAMPLE_RATE as u32,
            window_size: 512,
            provider: None,
            num_threads: Some(1),
            debug: false,
        };
        let vad = sherpa_rs::silero_vad::SileroVad::new(config, 30.0)
            .map_err(|error| anyhow::anyhow!("opening Silero VAD: {error}"))?;
        Ok(Self {
            vad,
            smoother: VadSmoother::new(
                std::time::Duration::from_millis(100),
                std::time::Duration::from_millis(350),
            ),
            voiced: Vec::new(),
        })
    }

    /// Feeds one microphone chunk, returns a gated 0..1 RMS level, and keeps
    /// only sherpa's voiced segments for the eventual transcription.
    pub fn push(&mut self, samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        self.vad.accept_waveform(samples.to_vec());
        let meter_open = self.smoother.push(self.vad.is_speech(), samples.len());
        self.drain_segments();
        if meter_open {
            rms(samples)
        } else {
            0.0
        }
    }

    /// Flushes sherpa's hangover and returns the audio with leading/trailing
    /// silence removed by the VAD.
    pub fn finish(mut self) -> Vec<f32> {
        self.vad.flush();
        self.drain_segments();
        self.voiced
    }

    fn drain_segments(&mut self) {
        while !self.vad.is_empty() {
            let segment = self.vad.front();
            self.voiced.extend_from_slice(&segment.samples);
            self.vad.pop();
        }
    }
}

fn rms(samples: &[f32]) -> f32 {
    let mean = samples.iter().map(|sample| sample * sample).sum::<f32>()
        / samples.len().max(1) as f32;
    (mean.sqrt() * 4.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_stays_closed_until_onset_then_opens() {
        let mut gate = VadSmoother::new(
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(350),
        );
        assert!(!gate.push(true, 800));
        assert!(gate.push(true, 800));
    }

    #[test]
    fn hangover_keeps_meter_open_for_short_gap_and_then_closes() {
        let mut gate = VadSmoother::new(
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(200),
        );
        assert!(gate.push(true, 1_000));
        assert!(gate.push(false, 1_000));
        assert!(!gate.push(false, 2_500));
        assert!(!gate.active());
    }

    #[test]
    fn silent_input_never_opens_the_gate() {
        let mut gate = VadSmoother::new(
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis(350),
        );
        for _ in 0..20 {
            assert!(!gate.push(false, 800));
        }
    }
}
