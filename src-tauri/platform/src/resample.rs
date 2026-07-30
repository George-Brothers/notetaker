//! Getting device audio to the one sample rate the pipeline accepts.
//!
//! `notetaker_core::capture::SAMPLE_RATE` is 16 kHz and
//! `pipeline::audio::load_mono_16k` rejects anything else, so this is not
//! optional polish — no real capture device offers 16 kHz. WASAPI mix formats
//! are 44.1 or 48 kHz, ScreenCaptureKit is 48 kHz, and microphones are
//! whatever they are.
//!
//! Doing it properly matters more than it looks. Dropping every third sample
//! to get from 48 kHz to 16 kHz *works*, in the sense that it produces a file
//! of the right length — but everything above 8 kHz folds back down into the
//! speech band as aliasing noise, and the only symptom is that transcription
//! quality is mysteriously worse than the bake-off promised. So this uses
//! `rubato`'s band-limited sinc resampler, and the tests below assert on
//! frequency content rather than on buffer lengths.
//!
//! Pure Rust and platform-independent on purpose: every line here is tested on
//! Linux, which shrinks the part of the capture path that can only be checked
//! on real hardware down to "does the OS hand us the bytes it said it would".

use anyhow::{Context, Result};
use rubato::{
    Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Input frames consumed per resampler pass. A compromise: large enough that
/// the sinc filter is doing useful work per call, small enough that a `read()`
/// draining a short buffer still gets output rather than holding everything
/// back until the next one. At 48 kHz this is ~21 ms.
const CHUNK: usize = 1024;

/// Resamples a mono `f32` stream from one rate to another, across calls.
///
/// Feed it whatever arrives with [`push`](Self::push) and take whatever is
/// ready with [`drain`](Self::drain). Input that does not fill a whole
/// [`CHUNK`] is held until it does, so no samples are lost at buffer
/// boundaries — the usual source of a periodic click every few milliseconds.
pub struct Resampler {
    /// `None` when input and output rates match, which makes this a
    /// passthrough. Worth special-casing: it is the only case where we can
    /// promise the samples are bit-identical.
    inner: Option<SincFixedIn<f32>>,
    pending: Vec<f32>,
    from_hz: u32,
    to_hz: u32,
    /// Scratch output buffer, reused so a steady stream stops allocating.
    scratch: Vec<Vec<f32>>,
}

impl Resampler {
    /// Builds a resampler from `from_hz` to `to_hz`.
    ///
    /// Both rates must be non-zero; a device reporting a zero sample rate is
    /// broken and the ratio would be meaningless.
    pub fn new(from_hz: u32, to_hz: u32) -> Result<Self> {
        anyhow::ensure!(
            from_hz > 0 && to_hz > 0,
            "cannot resample between {from_hz} Hz and {to_hz} Hz"
        );
        let inner = if from_hz == to_hz {
            None
        } else {
            let params = SincInterpolationParameters {
                // 256 taps is generous for a 3:1 downsample and cheap at these
                // rates; this runs once per recording, not per frame.
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 256,
                window: WindowFunction::BlackmanHarris2,
            };
            Some(
                SincFixedIn::<f32>::new(to_hz as f64 / from_hz as f64, 1.0, params, CHUNK, 1)
                    .with_context(|| format!("building a {from_hz} Hz -> {to_hz} Hz resampler"))?,
            )
        };
        // rubato writes into caller-owned buffers and refuses to grow them, so
        // they must be allocated at its stated maximum output size up front —
        // an empty `Vec` is rejected rather than resized.
        let scratch = match inner.as_ref() {
            Some(r) => r.output_buffer_allocate(true),
            None => vec![Vec::new()],
        };
        Ok(Self {
            inner,
            pending: Vec::with_capacity(CHUNK * 2),
            from_hz,
            to_hz,
            scratch,
        })
    }

    pub fn from_hz(&self) -> u32 {
        self.from_hz
    }

    pub fn to_hz(&self) -> u32 {
        self.to_hz
    }

    /// True when input and output rates match and samples pass through
    /// untouched.
    pub fn is_passthrough(&self) -> bool {
        self.inner.is_none()
    }

    /// Accepts input samples. Cheap; the work happens in [`drain`](Self::drain).
    pub fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
    }

    /// Appends every fully-resampled sample available to `out`.
    ///
    /// Leaves a partial chunk buffered for next time. Returns how many samples
    /// were appended.
    pub fn drain(&mut self, out: &mut Vec<f32>) -> Result<usize> {
        let Some(inner) = self.inner.as_mut() else {
            let n = self.pending.len();
            out.append(&mut self.pending);
            return Ok(n);
        };

        let mut written = 0;
        while self.pending.len() >= CHUNK {
            let input = [&self.pending[..CHUNK]];
            let produced = inner
                .process_into_buffer(&input, &mut self.scratch, None)
                .context("resampling captured audio")?;
            // `process_into_buffer` reports (frames_read, frames_written).
            let frames_out = produced.1;
            out.extend_from_slice(&self.scratch[0][..frames_out]);
            written += frames_out;
            self.pending.drain(..CHUNK);
        }
        Ok(written)
    }

    /// Samples accepted but not yet resampled. Only useful for assertions and
    /// diagnostics.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` of a sine at `freq_hz`, sampled at `rate`.
    fn sine(freq_hz: f32, rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (t * freq_hz * std::f32::consts::TAU).sin() * 0.5
            })
            .collect()
    }

    /// Estimates the dominant frequency by counting zero crossings. Crude, but
    /// it needs no FFT dependency and it is entirely sufficient to tell a
    /// correctly resampled 440 Hz tone from an aliased one.
    fn dominant_freq(samples: &[f32], rate: u32) -> f32 {
        let mut crossings = 0;
        for w in samples.windows(2) {
            if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
                crossings += 1;
            }
        }
        let secs = samples.len() as f32 / rate as f32;
        crossings as f32 / 2.0 / secs
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Runs a whole signal through in `chunk`-sized pushes, the way a capture
    /// loop actually delivers it.
    fn run(r: &mut Resampler, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for piece in input.chunks(chunk) {
            r.push(piece);
            r.drain(&mut out).unwrap();
        }
        out
    }

    // --- construction ---------------------------------------------------

    #[test]
    fn equal_rates_is_a_passthrough() {
        let r = Resampler::new(16_000, 16_000).unwrap();
        assert!(r.is_passthrough());
    }

    #[test]
    fn differing_rates_is_not_a_passthrough() {
        assert!(!Resampler::new(48_000, 16_000).unwrap().is_passthrough());
    }

    #[test]
    fn zero_rate_is_an_error_not_a_panic() {
        assert!(Resampler::new(0, 16_000).is_err());
        assert!(Resampler::new(48_000, 0).is_err());
    }

    #[test]
    fn passthrough_returns_samples_unchanged_and_in_order() {
        let mut r = Resampler::new(16_000, 16_000).unwrap();
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let out = run(&mut r, &input, 7);
        assert_eq!(out, input);
    }

    // --- rate correctness -----------------------------------------------

    /// 48 kHz -> 16 kHz must produce close to one third as many samples. The
    /// shortfall is the tail still buffered inside the sinc filter, which is
    /// bounded and does not grow with input length.
    #[test]
    fn downsampling_48k_to_16k_produces_about_a_third_as_many_samples() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let input = sine(440.0, 48_000, 2.0);
        let out = run(&mut r, &input, 480);
        let expected = input.len() / 3;
        let drift = (out.len() as f32 - expected as f32).abs() / expected as f32;
        assert!(
            drift < 0.02,
            "expected ~{expected} samples, got {} ({:.1}% off)",
            out.len(),
            drift * 100.0
        );
    }

    /// 44.1 kHz is the other rate real hardware hands us, and unlike 48 kHz it
    /// is not an integer ratio — the case a naive decimator cannot handle at
    /// all.
    #[test]
    fn downsampling_44k1_to_16k_produces_the_right_sample_count() {
        let mut r = Resampler::new(44_100, 16_000).unwrap();
        let input = sine(440.0, 44_100, 2.0);
        let out = run(&mut r, &input, 441);
        let expected = (input.len() as f64 * 16_000.0 / 44_100.0) as usize;
        let drift = (out.len() as f32 - expected as f32).abs() / expected as f32;
        assert!(
            drift < 0.02,
            "expected ~{expected}, got {} ({:.1}% off)",
            out.len(),
            drift * 100.0
        );
    }

    /// The one that actually matters: a 440 Hz tone must still be 440 Hz after
    /// resampling. A stride or ratio mistake shifts the pitch, and a file that
    /// plays at the wrong speed transcribes to nonsense.
    #[test]
    fn a_440hz_tone_is_still_440hz_after_resampling() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let out = run(&mut r, &sine(440.0, 48_000, 2.0), 480);
        let freq = dominant_freq(&out, 16_000);
        assert!(
            (freq - 440.0).abs() < 10.0,
            "expected ~440 Hz, measured {freq:.1} Hz"
        );
    }

    #[test]
    fn a_1khz_tone_is_still_1khz_after_resampling_from_44k1() {
        let mut r = Resampler::new(44_100, 16_000).unwrap();
        let out = run(&mut r, &sine(1_000.0, 44_100, 2.0), 441);
        let freq = dominant_freq(&out, 16_000);
        assert!(
            (freq - 1_000.0).abs() < 20.0,
            "expected ~1000 Hz, measured {freq:.1} Hz"
        );
    }

    /// The anti-aliasing assertion, and the reason `rubato` is here rather
    /// than a hand-rolled decimator. A 15 kHz tone is above the 8 kHz Nyquist
    /// limit of a 16 kHz stream, so it cannot be represented and must be
    /// filtered away. Naive decimation would instead fold it down to 1 kHz —
    /// a loud tone sitting right in the middle of the speech band.
    #[test]
    fn a_tone_above_the_output_nyquist_is_filtered_out_not_aliased() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let out = run(&mut r, &sine(15_000.0, 48_000, 1.0), 480);
        let level = rms(&out);
        assert!(
            level < 0.02,
            "15 kHz tone survived resampling at RMS {level:.4} — it has aliased \
             into the speech band instead of being filtered out"
        );
    }

    /// ...and the control for that test: a tone comfortably inside the band
    /// must come through at close to full strength, proving the filter is not
    /// simply attenuating everything.
    #[test]
    fn a_tone_below_the_output_nyquist_survives_at_full_strength() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let input = sine(1_000.0, 48_000, 1.0);
        let out = run(&mut r, &input, 480);
        let (before, after) = (rms(&input), rms(&out));
        assert!(
            (after / before) > 0.9,
            "1 kHz tone lost too much level: {before:.4} -> {after:.4}"
        );
    }

    // --- chunk-boundary behaviour ---------------------------------------

    /// Delivery size must not change the result. A capture callback hands over
    /// whatever the device felt like giving it, so identical audio arriving in
    /// different-sized pieces has to resample identically — otherwise there is
    /// a click at every buffer boundary.
    #[test]
    fn output_is_independent_of_the_size_of_the_pushes() {
        let input = sine(440.0, 48_000, 1.0);
        let mut a = Resampler::new(48_000, 16_000).unwrap();
        let mut b = Resampler::new(48_000, 16_000).unwrap();
        let out_small = run(&mut a, &input, 33); // awkward, non-aligned
        let out_large = run(&mut b, &input, 4096);
        assert_eq!(
            out_small.len(),
            out_large.len(),
            "different push sizes produced different amounts of audio"
        );
        for (i, (x, y)) in out_small.iter().zip(out_large.iter()).enumerate() {
            assert!(
                (x - y).abs() < 1e-4,
                "sample {i} differs between push sizes: {x} vs {y}"
            );
        }
    }

    #[test]
    fn input_shorter_than_a_chunk_is_buffered_not_dropped() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let mut out = Vec::new();
        r.push(&sine(440.0, 48_000, 0.001)); // ~48 samples, far under CHUNK
        r.drain(&mut out).unwrap();
        assert!(out.is_empty(), "should hold back a partial chunk");
        assert_eq!(r.pending_len(), 48);

        // Once enough arrives, it comes out.
        r.push(&sine(440.0, 48_000, 0.1));
        r.drain(&mut out).unwrap();
        assert!(!out.is_empty(), "buffered audio never came out");
    }

    #[test]
    fn draining_with_nothing_pushed_is_a_no_op() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let mut out = Vec::new();
        assert_eq!(r.drain(&mut out).unwrap(), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn drain_appends_to_an_existing_buffer() {
        let mut r = Resampler::new(16_000, 16_000).unwrap();
        let mut out = vec![42.0];
        r.push(&[1.0, 2.0]);
        r.drain(&mut out).unwrap();
        assert_eq!(out, vec![42.0, 1.0, 2.0]);
    }

    /// Silence in must be silence out — no DC offset or filter ringing
    /// injected by the resampler itself.
    #[test]
    fn silence_resamples_to_silence() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let out = run(&mut r, &vec![0.0f32; 48_000], 480);
        assert!(
            out.iter().all(|s| s.abs() < 1e-6),
            "resampler injected signal into silence"
        );
    }

    /// Output must stay inside the range a WAV file can hold. The sinc filter
    /// overshoots slightly on transients, so this allows a little headroom
    /// while still catching a scaling error.
    #[test]
    fn output_stays_in_range_for_a_full_scale_input() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let loud: Vec<f32> = sine(300.0, 48_000, 0.5).iter().map(|s| s * 2.0).collect();
        let out = run(&mut r, &loud, 480);
        assert!(
            out.iter().all(|s| s.abs() < 1.3),
            "resampler produced wildly out-of-range samples"
        );
    }

    /// Not a path we expect, but an 8 kHz headset exists and must not break.
    ///
    /// Asserted against *consumed* input rather than total input. Whatever has
    /// not yet filled a whole [`CHUNK`] is still buffered by design — the
    /// `input_shorter_than_a_chunk_is_buffered_not_dropped` test above pins
    /// that — and with a short signal that remainder is a large fraction of the
    /// output. Measuring against total input would make this test a statement
    /// about `CHUNK` rather than about the resampling ratio.
    #[test]
    fn upsampling_also_works() {
        let mut r = Resampler::new(8_000, 16_000).unwrap();
        let input = sine(440.0, 8_000, 1.0);
        let out = run(&mut r, &input, 160);

        let consumed = input.len() - r.pending_len();
        let expected = consumed * 2;
        let drift = (out.len() as f32 - expected as f32).abs() / expected as f32;
        assert!(
            drift < 0.02,
            "consumed {consumed} samples so expected ~{expected} out, got {}",
            out.len()
        );
        assert!(
            r.pending_len() < CHUNK,
            "more than a chunk left buffered: {}",
            r.pending_len()
        );

        let freq = dominant_freq(&out, 16_000);
        assert!((freq - 440.0).abs() < 10.0, "measured {freq:.1} Hz");
    }

    /// The same invariant stated directly, at the rate we actually ship: output
    /// length tracks *consumed* input times the ratio, and no more than one
    /// chunk is ever held back.
    #[test]
    fn output_length_tracks_consumed_input_times_the_ratio() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let input = sine(440.0, 48_000, 3.0);
        let out = run(&mut r, &input, 512);

        let consumed = input.len() - r.pending_len();
        let expected = consumed / 3;
        let drift = (out.len() as f32 - expected as f32).abs() / expected as f32;
        assert!(
            drift < 0.01,
            "consumed {consumed}, expected ~{expected}, got {}",
            out.len()
        );
        assert!(r.pending_len() < CHUNK);
    }

    #[test]
    fn reports_its_rates() {
        let r = Resampler::new(44_100, 16_000).unwrap();
        assert_eq!(r.from_hz(), 44_100);
        assert_eq!(r.to_hz(), 16_000);
    }
}
