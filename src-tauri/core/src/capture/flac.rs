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
//! half-written temporary FLAC away. A verified FLAC is published only after
//! the decode check, so a crash cannot expose a partial derived file as if it
//! were complete.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;

use crate::capture::{MIC_TRACK, SAMPLE_RATE, SYSTEM_TRACK};
use crate::pipeline::audio::load_mono_16k;
use crate::storage::{CompressionStatus, Mode, RecordingRef, Store};

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
/// What a finalization actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finalized {
    /// The verified lossless copy of the audio.
    pub flac: PathBuf,
    /// The original WAV that could not be removed after a successful encode.
    pub wav_kept: Option<PathBuf>,
}

/// Returns the verified FLAC, plus whether the original WAV had to remain.
/// Errors leave `wav_path` untouched: an unreadable input, a FLAC that will
/// not write, and a FLAC that decodes to anything other than the source audio
/// all end here with the WAV still on disk.
pub fn finalize_to_flac(wav_path: &Path, keep_wav: bool) -> Result<Finalized> {
    finalize_with_remove(wav_path, keep_wav, remove_with_one_retry)
}

/// Implementation seam for a cleanup failure that cannot be reproduced
/// reliably on Unix: Windows refuses to unlink an open audio handle while
/// Unix permits it.
fn finalize_with_remove<F>(wav_path: &Path, keep_wav: bool, mut remove: F) -> Result<Finalized>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let flac_path = wav_path.with_extension("flac");
    let temporary_flac = wav_path.with_file_name(format!(
        ".{}.tmp-{}.flac",
        wav_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("audio"),
        uuid::Uuid::new_v4()
    ));

    let source = load_mono_16k(wav_path)
        .with_context(|| format!("reading {} to encode it as FLAC", wav_path.display()))?;
    if source.is_empty() {
        bail!(
            "{} holds no audio, so there is nothing to encode",
            wav_path.display()
        );
    }

    if let Err(e) = encode(&source, &temporary_flac)
        .and_then(|()| verify(&source, &temporary_flac))
        .and_then(|()| replace_verified_flac(&temporary_flac, &flac_path))
    {
        // Whatever landed is not trustworthy, and leaving it behind would let
        // the next recovery sweep mistake it for a finished track.
        if let Err(cleanup) = remove_if_file(&temporary_flac) {
            log::warn!(
                "could not remove the unusable temporary FLAC {}: {cleanup:#}",
                temporary_flac.display()
            );
        }
        return Err(e);
    }

    // The encode is verified: the audio is safe in the FLAC. Whether the WAV
    // goes away is a disk-space question, and must never be reported as a
    // lost recording. Windows can refuse while the capture handle is closing;
    // anything still open after the retry is reported to the caller.
    let wav_kept = if !keep_wav {
        match remove(wav_path) {
            Ok(()) => None,
            Err(e) => {
                log::warn!(
                    "keeping {} as WAV after a verified FLAC: {e}",
                    wav_path.display()
                );
                Some(wav_path.to_path_buf())
            }
        }
    } else {
        None
    };
    Ok(Finalized {
        flac: flac_path,
        wav_kept,
    })
}

/// Publishes a verified FLAC in one same-directory rename. The destination is
/// derived data, so replacing an older copy is safe only after the new copy
/// has already decoded byte-for-byte against the WAV.
fn replace_verified_flac(temporary: &Path, destination: &Path) -> Result<()> {
    crate::storage::replace_atomically(temporary, destination)
        .with_context(|| format!("publishing verified FLAC {}", destination.display()))?;

    if let Some(parent) = destination.parent() {
        #[cfg(unix)]
        if let Err(error) = std::fs::File::open(parent).and_then(|directory| directory.sync_all()) {
            log::warn!("could not sync FLAC directory {}: {error}", parent.display());
        }
    }
    Ok(())
}

/// Finalizes the audio tracks for a queued recording. This is deliberately a
/// queue operation, never a capture/Stop operation: the WAV is already
/// durable before this function can run, and a failed encode leaves that WAV
/// available for the pipeline and a later retry.
pub fn finalize_recording_tracks(
    store: &Store,
    recording: &RecordingRef,
    keep_wav: bool,
) -> Result<RecordingRef> {
    let mut rec = recording.clone();
    let expected = match rec.meta.mode {
        Mode::InPerson => [MIC_TRACK, ""],
        Mode::Meeting => [MIC_TRACK, SYSTEM_TRACK],
    };
    let mut failures = Vec::new();
    let mut durations = Vec::new();

    for stem in expected.into_iter().filter(|stem| !stem.is_empty()) {
        let wav = rec.dir.join(format!("{stem}.wav"));
        let flac = rec.dir.join(format!("{stem}.flac"));
        if wav.is_file() {
            match finalize_to_flac(&wav, keep_wav) {
                Ok(finalized) => {
                    if let Some(kept_wav) = finalized.wav_kept {
                        failures.push(format!(
                            "{stem}: verified FLAC published but WAV cleanup failed ({})",
                            kept_wav.display()
                        ));
                    }
                    match load_mono_16k(&finalized.flac) {
                        Ok(samples) => durations.push(samples.len() as f64 / SAMPLE_RATE as f64),
                        Err(error) => failures.push(format!(
                            "{stem}: published FLAC could not be decoded ({error:#})"
                        )),
                    }
                }
                Err(error) => failures.push(format!("{stem}: {error:#}")),
            }
        } else if flac.is_file() {
            match load_mono_16k(&flac) {
                Ok(samples) => durations.push(samples.len() as f64 / SAMPLE_RATE as f64),
                Err(error) => failures.push(format!("{stem}: {error:#}")),
            }
        } else {
            failures.push(format!("{stem}: no WAV or FLAC track exists"));
        }
    }

    if let Some(longest) = durations.into_iter().reduce(f64::max) {
        rec.meta.duration_s = longest;
    }
    if failures.is_empty() {
        rec.meta.compression = CompressionStatus::Complete;
        rec.meta.compression_error = None;
    } else {
        rec.meta.compression = CompressionStatus::Failed;
        rec.meta.compression_error = Some(failures.join("; "));
        log::warn!(
            "derived audio for {} needs retry: {}",
            rec.meta.id,
            rec.meta.compression_error.as_deref().unwrap_or_default()
        );
    }
    store.save_meta(&rec)?;
    Ok(rec)
}

/// Removes a file, retrying once after a short pause.
///
/// A capture handle that is only just closing makes the first removal fail on
/// Windows. Unix succeeds on the first attempt, so it never waits.
fn remove_with_one_retry(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(250));
            std::fs::remove_file(path)
        }
    }
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
        let flac = finalize_to_flac(&wav, true).unwrap().flac;
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
            let flac = finalize_to_flac(&wav, true).unwrap().flac;
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
        let flac = finalize_to_flac(&dropped, false).unwrap().flac;
        assert!(
            !dropped.exists(),
            "keep_wav = false must reclaim the space once the FLAC is verified"
        );
        assert_eq!(load_mono_16k(&flac).unwrap().len(), frames.len());
    }

    #[test]
    fn a_wav_that_cannot_be_deleted_is_still_a_successful_encode() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        write_wav(&wav, &tone(0.3, 0.4));

        let done = finalize_with_remove(&wav, false, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .expect("a verified FLAC is a success whatever the delete did");

        assert!(done.flac.exists(), "the FLAC must survive");
        assert_eq!(
            done.wav_kept.as_deref(),
            Some(wav.as_path()),
            "the surviving WAV must be reported, not silently left"
        );
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
