//! Lossless WAV -> FLAC finalize, verified by decoding before the WAV is
//! deleted.
//!
//! Capture writes WAV because a WAV can be appended to and left valid on disk
//! mid-recording; FLAC is what a finished recording should sit in, at roughly
//! half the size for bit-identical audio. This module is the bridge, and its
//! whole design follows from one asymmetry: **wasting a few hundred megabytes
//! is annoying, losing a lecture is not recoverable at all.** So the WAV is
//! never removed on the encoder's say-so. The FLAC is written, decoded back
//! through the same loader the transcription stage uses
//! ([`load_mono_16k`]), and compared sample for sample against the source.
//! Only an exact match earns the delete, and even then only if the caller
//! asked for it — `keep_wav` is a user setting, so the decision belongs to
//! the caller rather than to this module reading settings behind its back.
//!
//! A failed encode leaves the WAV exactly where it was and takes the
//! half-written FLAC away, because a stray `.flac` beside a `.wav` is how
//! [`crate::capture::recover`] decides a track is already finished.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;

use crate::capture::SAMPLE_RATE;
use crate::pipeline::audio::load_mono_16k;

/// The divisor `pipeline::audio` uses to turn 16-bit PCM into `f32`. Going
/// back the other way with the same constant is exact — it is a power of two,
/// and every 16-bit value fits an `f32` mantissa — which is what lets the
/// verification below demand equality rather than a tolerance.
const FULL_SCALE: f32 = 32768.0;

/// Bit depth of the encoded stream. Matches capture, and matches what
/// `load_mono_16k` expects to read back.
const BITS_PER_SAMPLE: usize = 16;

/// Encodes `wav_path` to a FLAC beside it, verifies the result decodes to
/// exactly the same audio, and only then removes the WAV — and only if
/// `keep_wav` is false.
///
/// Returns the path of the FLAC. Errors leave `wav_path` untouched: an
/// unreadable input, a FLAC that will not write, and a FLAC that decodes to
/// anything other than the source audio all end here with the WAV still on
/// disk.
pub fn finalize_to_flac(wav_path: &Path, keep_wav: bool) -> Result<PathBuf> {
    let flac_path = wav_path.with_extension("flac");

    let source = load_mono_16k(wav_path)
        .with_context(|| format!("reading {} to encode it as FLAC", wav_path.display()))?;
    if source.is_empty() {
        bail!(
            "{} holds no audio, so there is nothing to encode",
            wav_path.display()
        );
    }

    if let Err(e) = encode(&source, &flac_path).and_then(|()| verify(&source, &flac_path)) {
        // Whatever landed is not trustworthy, and leaving it behind would let
        // the next recovery sweep mistake it for a finished track.
        if let Err(cleanup) = remove_if_file(&flac_path) {
            log::warn!(
                "could not remove the unusable {}: {cleanup:#}",
                flac_path.display()
            );
        }
        return Err(e);
    }

    if !keep_wav {
        std::fs::remove_file(wav_path).with_context(|| {
            format!(
                "removing {} after its FLAC was verified",
                wav_path.display()
            )
        })?;
    }
    Ok(flac_path)
}

/// Writes `samples` to `flac_path` as 16 kHz mono FLAC.
fn encode(samples: &[f32], flac_path: &Path) -> Result<()> {
    // flacenc takes `i32` in the *bit depth's* range, not full `i32` range.
    // Clamping matters even though capture already clamps: this also runs over
    // files repaired from a crash, which nobody promised were well-formed.
    let pcm: Vec<i32> = samples
        .iter()
        .map(|s| {
            (s * FULL_SCALE)
                .round()
                .clamp(-FULL_SCALE, FULL_SCALE - 1.0) as i32
        })
        .collect();

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| anyhow!("the FLAC encoder rejected its own default settings: {e}"))?;
    let block_size = config.block_size;
    let source =
        flacenc::source::MemSource::from_samples(&pcm, 1, BITS_PER_SAMPLE, SAMPLE_RATE as usize);

    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, block_size)
        .map_err(|e| anyhow!("encoding {}: {e}", flac_path.display()))?;

    // flacenc reports the *shortest* frame — always the final, partial one —
    // as the stream's minimum block size. A decoder reads
    // `min_block_size != max_block_size` as "this stream has variable block
    // sizes", then rejects every one of flacenc's fixed-block-size frame
    // headers and gives up looking for audio at all. libFLAC and ffmpeg both
    // leave min == max and simply do not count the final short block; matching
    // them is what makes the file readable by `symphonia`, and so by the
    // transcription stage, rather than merely well-formed on paper.
    stream
        .stream_info_mut()
        .set_block_sizes(block_size, block_size)
        .map_err(|e| anyhow!("declaring the block size of {}: {e}", flac_path.display()))?;

    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow!("serializing {}: {e}", flac_path.display()))?;

    std::fs::write(flac_path, sink.as_slice())
        .with_context(|| format!("writing {}", flac_path.display()))
}

/// Decodes `flac_path` back through the pipeline's own loader and insists it
/// matches `source` exactly.
///
/// Exactly, not approximately: FLAC is lossless by definition, so any drift at
/// all means the encoder, the decoder, or the file is wrong — and this check is
/// the only thing standing between that and a deleted WAV.
fn verify(source: &[f32], flac_path: &Path) -> Result<()> {
    let decoded = load_mono_16k(flac_path)
        .with_context(|| format!("decoding {} to verify it", flac_path.display()))?;

    if decoded.len() != source.len() {
        bail!(
            "{} decoded to {} samples but the source has {} — refusing to trust it",
            flac_path.display(),
            decoded.len(),
            source.len()
        );
    }
    if let Some(i) = decoded.iter().zip(source).position(|(a, b)| a != b) {
        bail!(
            "{} is not a lossless copy: sample {i} decoded as {} but was {} — refusing to \
             trust it",
            flac_path.display(),
            decoded[i],
            source[i]
        );
    }
    Ok(())
}

/// Deletes `path` if it is a file. An absent path is success — the point is
/// only that no unusable FLAC is left behind.
fn remove_if_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::track::TrackWriter;

    /// `secs` seconds of a 440 Hz tone — real, varied audio, so a decoder that
    /// silently produced silence or a constant could not pass.
    fn tone(secs: f64, amp: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * amp
            })
            .collect()
    }

    /// A finished, valid track on disk.
    fn write_wav(path: &Path, frames: &[f32]) {
        let mut track = TrackWriter::create(path).unwrap();
        track.write(frames).unwrap();
        track.finalize().unwrap();
    }

    #[test]
    fn wav_to_flac_round_trips_every_sample_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        write_wav(&wav, &tone(1.0, 0.6));

        let before = load_mono_16k(&wav).unwrap();
        let flac = finalize_to_flac(&wav, true).unwrap();
        let after = load_mono_16k(&flac).unwrap();

        assert_eq!(flac, dir.path().join("audio-mic.flac"));
        assert_eq!(after.len(), before.len(), "the FLAC lost or gained audio");
        assert_eq!(
            after, before,
            "FLAC is lossless — any drift at all means the encode is not trustworthy"
        );
        assert!(
            before.iter().any(|s| s.abs() > 0.1),
            "the fixture must be real audio, or this test proves nothing"
        );
    }

    /// Guards the `set_block_sizes` call in [`encode`]. A recording almost
    /// never ends on a block boundary, and if the stream advertises the short
    /// final block as its minimum, decoders read the stream as
    /// variable-block-size and refuse every frame in it — the file looks fine
    /// and contains no audio anyone can get at.
    #[test]
    fn recordings_that_do_not_end_on_a_block_boundary_still_decode() {
        let dir = tempfile::tempdir().unwrap();

        // 3200 samples (shorter than one block), 8192 (exactly two), and
        // 16000 (three blocks and a stub).
        for (name, n) in [("short", 3200), ("exact", 8192), ("ragged", 16000)] {
            let frames: Vec<f32> = tone(1.0, 0.6).into_iter().take(n).collect();
            let wav = dir.path().join(format!("{name}.wav"));
            write_wav(&wav, &frames);

            let before = load_mono_16k(&wav).unwrap();
            let flac = finalize_to_flac(&wav, true).unwrap();
            assert_eq!(load_mono_16k(&flac).unwrap(), before, "{name}");

            let head = std::fs::read(&flac).unwrap();
            let min = u16::from_be_bytes([head[8], head[9]]);
            let max = u16::from_be_bytes([head[10], head[11]]);
            assert_eq!(
                min, max,
                "{name}: a fixed-block-size stream must declare min == max"
            );
        }
    }

    #[test]
    fn keep_wav_decides_whether_the_source_survives_a_verified_encode() {
        let dir = tempfile::tempdir().unwrap();
        let frames = tone(0.3, 0.4);

        let kept = dir.path().join("kept.wav");
        write_wav(&kept, &frames);
        finalize_to_flac(&kept, true).unwrap();
        assert!(kept.exists(), "keep_wav = true must leave the wav alone");

        let dropped = dir.path().join("dropped.wav");
        write_wav(&dropped, &frames);
        let flac = finalize_to_flac(&dropped, false).unwrap();
        assert!(
            !dropped.exists(),
            "keep_wav = false must reclaim the space once the FLAC is verified"
        );
        assert_eq!(load_mono_16k(&flac).unwrap().len(), frames.len());
    }

    #[test]
    fn a_failed_encode_leaves_the_wav_in_place_even_when_keep_wav_is_false() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        write_wav(&wav, &tone(0.3, 0.4));
        let before = std::fs::read(&wav).unwrap();

        // Block the destination the crude, portable way: a directory where the
        // FLAC wants to be.
        std::fs::create_dir(dir.path().join("audio-mic.flac")).unwrap();

        let err = finalize_to_flac(&wav, false).unwrap_err();
        assert!(
            wav.exists(),
            "the recording must survive an encode that never worked: {err:#}"
        );
        assert_eq!(
            std::fs::read(&wav).unwrap(),
            before,
            "a failed encode must not modify the source audio"
        );
    }

    #[test]
    fn an_unreadable_input_is_reported_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        std::fs::write(&wav, b"this is not audio").unwrap();

        assert!(finalize_to_flac(&wav, false).is_err());
        assert_eq!(
            std::fs::read(&wav).unwrap(),
            b"this is not audio",
            "an input we could not even read must be left exactly as found"
        );
        assert!(
            !dir.path().join("audio-mic.flac").exists(),
            "no half-written FLAC may be left claiming this track is finished"
        );
    }

    #[test]
    fn an_empty_wav_is_reported_rather_than_encoded_away() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        write_wav(&wav, &[]);

        let err = finalize_to_flac(&wav, false).unwrap_err();
        assert!(format!("{err:#}").contains("no audio"), "{err:#}");
        assert!(
            wav.exists(),
            "even an empty recording is never deleted here"
        );
    }
}
