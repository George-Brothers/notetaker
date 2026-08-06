//! Turning whatever bytes the operating system hands us into mono `f32`.
//!
//! This is where the real bugs in a capture layer live — a wrong stride, the
//! wrong endianness, an off-by-one that reads half of the next frame — and
//! every one of them produces *audio*, not an error. Static, buzzing, or a
//! recording that plays at the wrong speed. The pipeline downstream cannot
//! tell the difference between that and a bad microphone.
//!
//! So none of it is written inside a platform callback where it can only be
//! tested on the hardware. It is pure functions over byte slices, tested on
//! Linux against hand-computed expectations.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/audio-actual/src/speaker/windows.rs`. See the NOTICE file.
//! The notable change is [`downmix`]: anarlog takes the first channel only,
//! which silently discards anything panned to the right. For a recording of a
//! meeting that is a real loss, so this averages the channels instead.

/// How a device lays out one sample in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// 32-bit IEEE float, `-1.0..=1.0`. What WASAPI mix formats almost always
    /// use, and what CoreAudio gives us.
    F32,
    /// 16-bit signed integer, little-endian.
    I16,
    /// 32-bit signed integer, little-endian.
    I32,
}

impl SampleFormat {
    /// Bytes occupied by a single sample of one channel.
    pub const fn width(self) -> usize {
        match self {
            SampleFormat::F32 => 4,
            SampleFormat::I16 => 2,
            SampleFormat::I32 => 4,
        }
    }
}

/// Converts one interleaved buffer to mono `f32`, appending to `out`.
///
/// A trailing partial frame is ignored rather than padded with silence: WASAPI
/// hands over whole frames, so a partial one means we have misread the format,
/// and inventing a sample would bury that. Returns the number of frames
/// written so a caller can notice the count drifting from what the device
/// claimed.
///
/// `channels == 0` yields nothing — a device reporting no channels is broken,
/// and dividing by it would panic in release as surely as debug.
pub fn to_mono_f32(
    data: &[u8],
    format: SampleFormat,
    channels: usize,
    out: &mut Vec<f32>,
) -> usize {
    if channels == 0 {
        return 0;
    }
    let frame_bytes = channels * format.width();
    let frames = data.len() / frame_bytes;
    out.reserve(frames);
    for f in 0..frames {
        let frame = &data[f * frame_bytes..(f + 1) * frame_bytes];
        out.push(downmix(frame, format, channels));
    }
    frames
}

/// Averages one interleaved frame's channels into a single sample.
///
/// Averaging rather than summing, so a stereo frame at full scale on both
/// channels stays at full scale instead of clipping at 2.0.
fn downmix(frame: &[u8], format: SampleFormat, channels: usize) -> f32 {
    let w = format.width();
    let mut sum = 0.0f32;
    for c in 0..channels {
        sum += decode(&frame[c * w..(c + 1) * w], format);
    }
    sum / channels as f32
}

/// Decodes exactly one sample. `bytes.len()` must be `format.width()`.
fn decode(bytes: &[u8], format: SampleFormat) -> f32 {
    match format {
        SampleFormat::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        // Divided by 32768 (2^15), not 32767: the range is -32768..=32767, so
        // this maps the most negative value to exactly -1.0 and the most
        // positive to just under it. Using 32767 would let -32768 become
        // -1.000031, which clips on the way to a WAV file.
        SampleFormat::I16 => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        SampleFormat::I32 => {
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 2_147_483_648.0
        }
    }
}

/// Downmixes an already-decoded interleaved `f32` buffer. This is the cpal
/// path — cpal hands over typed samples, so there are no bytes to decode.
pub fn interleaved_f32_to_mono(data: &[f32], channels: usize, out: &mut Vec<f32>) -> usize {
    if channels == 0 {
        return 0;
    }
    let frames = data.len() / channels;
    out.reserve(frames);
    for f in 0..frames {
        let frame = &data[f * channels..(f + 1) * channels];
        out.push(frame.iter().sum::<f32>() / channels as f32);
    }
    frames
}

/// Downmixes **planar** (non-interleaved) `f32` channels into mono.
///
/// This is the ScreenCaptureKit path, and it is a genuinely different memory
/// layout rather than a variation on one. An interleaved stereo buffer is
/// `LRLRLR`; ScreenCaptureKit hands over an `AudioBufferList` holding one
/// buffer *per channel* — `LLL` and `RRR` — so each plane arrives as its own
/// slice.
///
/// Getting this wrong is the exact failure [`to_mono_f32`]'s module docs warn
/// about. Feeding a planar buffer to [`interleaved_f32_to_mono`] does not error:
/// it reads `L[0]` and `L[1]` as if they were a left/right pair, and produces a
/// recording that is audible, half the intended length, and playing at double
/// speed. Nothing downstream can tell that from a misconfigured device, which
/// is why the two layouts get two functions instead of a `bool`.
///
/// Frames are taken as the length of the **shortest** plane. Channels of
/// unequal length mean we have misread the buffer list, and reading past the
/// end of the short one would be unsound; truncating keeps the audio correct
/// and lets the returned frame count show the drift.
///
/// No planes, or planes of zero length, yields nothing.
pub fn planar_f32_to_mono(planes: &[&[f32]], out: &mut Vec<f32>) -> usize {
    if planes.is_empty() {
        return 0;
    }
    let frames = planes.iter().map(|p| p.len()).min().unwrap_or(0);
    if frames == 0 {
        return 0;
    }

    out.reserve(frames);
    let channels = planes.len() as f32;
    for f in 0..frames {
        // Averaging, not summing — same reason as `downmix`: two channels at
        // full scale must stay at full scale rather than clipping at 2.0.
        let sum: f32 = planes.iter().map(|p| p[f]).sum();
        out.push(sum / channels);
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- widths ---------------------------------------------------------

    #[test]
    fn widths_match_the_formats() {
        assert_eq!(SampleFormat::F32.width(), 4);
        assert_eq!(SampleFormat::I16.width(), 2);
        assert_eq!(SampleFormat::I32.width(), 4);
    }

    // --- f32 ------------------------------------------------------------

    #[test]
    fn f32_mono_passes_values_through_unchanged() {
        let mut bytes = Vec::new();
        for v in [0.0f32, 0.5, -0.5, 1.0, -1.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        let frames = to_mono_f32(&bytes, SampleFormat::F32, 1, &mut out);
        assert_eq!(frames, 5);
        assert_eq!(out, vec![0.0, 0.5, -0.5, 1.0, -1.0]);
    }

    /// The bug this guards: reading the left channel twice, or reading across
    /// a frame boundary. Left and right differ in every frame so either
    /// mistake changes the answer.
    #[test]
    fn f32_stereo_averages_the_two_channels() {
        let frames: &[(f32, f32)] = &[(1.0, 0.0), (0.0, 1.0), (0.5, -0.5), (-1.0, 1.0)];
        let mut bytes = Vec::new();
        for (l, r) in frames {
            bytes.extend_from_slice(&l.to_le_bytes());
            bytes.extend_from_slice(&r.to_le_bytes());
        }
        let mut out = Vec::new();
        let n = to_mono_f32(&bytes, SampleFormat::F32, 2, &mut out);
        assert_eq!(n, 4);
        assert_eq!(out, vec![0.5, 0.5, 0.0, 0.0]);
    }

    /// A sound only on the right channel must survive. anarlog's
    /// first-channel-only approach returns silence here; that difference is
    /// the reason this function exists.
    #[test]
    fn audio_only_on_the_right_channel_is_not_lost() {
        let mut bytes = Vec::new();
        for _ in 0..3 {
            bytes.extend_from_slice(&0.0f32.to_le_bytes());
            bytes.extend_from_slice(&0.8f32.to_le_bytes());
        }
        let mut out = Vec::new();
        to_mono_f32(&bytes, SampleFormat::F32, 2, &mut out);
        assert!(
            out.iter().all(|s| *s > 0.3),
            "right-channel audio was dropped: {out:?}"
        );
    }

    /// Stereo at full scale on both channels must not clip.
    #[test]
    fn full_scale_stereo_stays_within_range() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        let mut out = Vec::new();
        to_mono_f32(&bytes, SampleFormat::F32, 2, &mut out);
        assert_eq!(out, vec![1.0]);
    }

    #[test]
    fn seven_point_one_averages_all_eight_channels() {
        let mut bytes = Vec::new();
        for c in 0..8 {
            bytes.extend_from_slice(&(c as f32 / 8.0).to_le_bytes());
        }
        let mut out = Vec::new();
        let n = to_mono_f32(&bytes, SampleFormat::F32, 8, &mut out);
        assert_eq!(n, 1);
        // mean of 0/8 .. 7/8
        assert!((out[0] - 0.4375).abs() < 1e-6, "got {}", out[0]);
    }

    // --- integer formats ------------------------------------------------

    #[test]
    fn i16_extremes_map_to_the_full_range_without_exceeding_it() {
        let mut bytes = Vec::new();
        for v in [0i16, i16::MAX, i16::MIN] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        to_mono_f32(&bytes, SampleFormat::I16, 1, &mut out);
        assert_eq!(out[0], 0.0);
        assert!(out[1] < 1.0 && out[1] > 0.999, "i16::MAX -> {}", out[1]);
        assert_eq!(out[2], -1.0, "i16::MIN must be exactly -1.0, not beyond it");
        assert!(
            out.iter().all(|s| (-1.0..=1.0).contains(s)),
            "out of range: {out:?}"
        );
    }

    #[test]
    fn i32_extremes_map_to_the_full_range_without_exceeding_it() {
        let mut bytes = Vec::new();
        for v in [0i32, i32::MAX, i32::MIN] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        to_mono_f32(&bytes, SampleFormat::I32, 1, &mut out);
        assert_eq!(out[0], 0.0);
        // `<= 1.0`, not `< 1.0`: an `f32` has a 24-bit mantissa, so `i32::MAX`
        // widens to exactly 2^31 and the quotient is exactly 1.0. That is in
        // range and correct — full scale is full scale.
        assert!(out[1] <= 1.0 && out[1] > 0.999, "i32::MAX -> {}", out[1]);
        assert_eq!(out[2], -1.0);
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    /// Little-endian is asserted explicitly: a byte-order mistake here is pure
    /// noise on the output, and would pass any test that only checked lengths.
    #[test]
    fn i16_is_read_little_endian() {
        // 0x0100 little-endian = 256
        let mut out = Vec::new();
        to_mono_f32(&[0x00, 0x01], SampleFormat::I16, 1, &mut out);
        assert_eq!(out[0], 256.0 / 32_768.0);
    }

    #[test]
    fn i16_stereo_averages_channels() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&16_384i16.to_le_bytes()); // +0.5
        bytes.extend_from_slice(&(-16_384i16).to_le_bytes()); // -0.5
        let mut out = Vec::new();
        to_mono_f32(&bytes, SampleFormat::I16, 2, &mut out);
        assert_eq!(out, vec![0.0]);
    }

    // --- partial and degenerate input -----------------------------------

    /// A trailing partial frame must be dropped, never padded. Padding would
    /// hide a format misread behind a plausible-looking buffer.
    #[test]
    fn trailing_partial_frame_is_dropped_not_padded() {
        // 2 channels of f32 = 8 bytes per frame; give 12 -> 1 whole frame.
        let bytes = vec![0u8; 12];
        let mut out = Vec::new();
        let n = to_mono_f32(&bytes, SampleFormat::F32, 2, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn zero_channels_yields_nothing_instead_of_dividing_by_zero() {
        let mut out = Vec::new();
        let n = to_mono_f32(&[0u8; 16], SampleFormat::F32, 0, &mut out);
        assert_eq!(n, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn empty_input_yields_nothing() {
        let mut out = Vec::new();
        assert_eq!(to_mono_f32(&[], SampleFormat::F32, 2, &mut out), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn output_is_appended_not_overwritten() {
        let mut out = vec![9.0];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0.25f32.to_le_bytes());
        to_mono_f32(&bytes, SampleFormat::F32, 1, &mut out);
        assert_eq!(out, vec![9.0, 0.25]);
    }

    // --- the cpal path --------------------------------------------------

    #[test]
    fn interleaved_f32_stereo_averages_channels() {
        let mut out = Vec::new();
        let n = interleaved_f32_to_mono(&[1.0, 0.0, 0.0, 1.0, 0.5, 0.5], 2, &mut out);
        assert_eq!(n, 3);
        assert_eq!(out, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn interleaved_f32_mono_is_a_passthrough() {
        let mut out = Vec::new();
        interleaved_f32_to_mono(&[0.1, -0.2, 0.3], 1, &mut out);
        assert_eq!(out, vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn interleaved_f32_drops_a_partial_frame() {
        let mut out = Vec::new();
        let n = interleaved_f32_to_mono(&[1.0, 1.0, 1.0], 2, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out, vec![1.0]);
    }

    #[test]
    fn interleaved_f32_zero_channels_is_safe() {
        let mut out = Vec::new();
        assert_eq!(interleaved_f32_to_mono(&[1.0, 2.0], 0, &mut out), 0);
    }

    // --- planar (ScreenCaptureKit) ---------------------------------------

    #[test]
    fn planar_f32_stereo_averages_the_two_planes() {
        let left = [1.0f32, 0.0, 0.5];
        let right = [0.0f32, 1.0, 0.5];
        let mut out = Vec::new();
        let n = planar_f32_to_mono(&[&left, &right], &mut out);
        assert_eq!(n, 3);
        assert_eq!(out, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn planar_f32_single_plane_is_a_passthrough() {
        let only = [0.1f32, -0.2, 0.3];
        let mut out = Vec::new();
        assert_eq!(planar_f32_to_mono(&[&only], &mut out), 3);
        assert_eq!(out, vec![0.1, -0.2, 0.3]);
    }

    /// The whole reason this function exists, stated as a test.
    ///
    /// Same bytes, two layouts, two different recordings — and neither one
    /// errors. Planar `LLL`/`RRR` read as interleaved comes out at half the
    /// length, which is a recording that plays at double speed.
    #[test]
    fn planar_and_interleaved_disagree_on_the_same_numbers() {
        let left = [1.0f32, 1.0, 1.0];
        let right = [0.0f32, 0.0, 0.0];

        let mut planar = Vec::new();
        planar_f32_to_mono(&[&left, &right], &mut planar);
        assert_eq!(planar, vec![0.5, 0.5, 0.5], "three frames at half scale");

        // The same six numbers laid end to end, misread as interleaved stereo.
        let flat: Vec<f32> = left.iter().chain(right.iter()).copied().collect();
        let mut wrong = Vec::new();
        interleaved_f32_to_mono(&flat, 2, &mut wrong);
        // Pairs (1,1), (1,0), (0,0) — three frames, but not the same three.
        assert_eq!(wrong, vec![1.0, 0.5, 0.0], "three frames, but the wrong ones");

        assert_ne!(planar, wrong);
    }

    /// Unequal planes mean the buffer list was misread. Truncating to the
    /// shortest keeps this in bounds; the frame count is what shows the drift.
    #[test]
    fn planar_f32_truncates_to_the_shortest_plane() {
        let left = [1.0f32, 1.0, 1.0, 1.0];
        let right = [1.0f32, 1.0];
        let mut out = Vec::new();
        assert_eq!(planar_f32_to_mono(&[&left, &right], &mut out), 2);
        assert_eq!(out, vec![1.0, 1.0]);
    }

    #[test]
    fn planar_f32_no_planes_or_empty_planes_are_safe() {
        let mut out = Vec::new();
        assert_eq!(planar_f32_to_mono(&[], &mut out), 0);

        let empty: [f32; 0] = [];
        let full = [1.0f32, 2.0];
        assert_eq!(planar_f32_to_mono(&[&empty], &mut out), 0);
        assert_eq!(planar_f32_to_mono(&[&full, &empty], &mut out), 0);
        assert!(out.is_empty());
    }

    /// Four channels average, rather than the first channel winning. Some
    /// aggregate devices present more than two.
    #[test]
    fn planar_f32_handles_more_than_two_channels() {
        let a = [1.0f32];
        let b = [0.0f32];
        let c = [1.0f32];
        let d = [0.0f32];
        let mut out = Vec::new();
        planar_f32_to_mono(&[&a, &b, &c, &d], &mut out);
        assert_eq!(out, vec![0.5]);
    }
}
