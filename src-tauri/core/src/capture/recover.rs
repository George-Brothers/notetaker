//! Recovering recordings whose capture died mid-write: repair the WAV header,
//! finalize, and put them back in the queue.
//!
//! [`crate::capture::track::TrackWriter`] keeps a recording valid on disk by
//! rewriting its WAV header every [`FLUSH_INTERVAL_SECS`], but the audio
//! itself reaches the file continuously. A power cut therefore leaves a file
//! whose header *understates* what it holds — up to a flush interval of real
//! audio sitting past the end of the declared `data` chunk, invisible to every
//! reader in the world. That gap is what this module exists to close.
//!
//! One rule governs the whole file: **nothing here ever deletes or truncates a
//! recording.** A file we cannot make sense of is left byte for byte as found,
//! with a plain-English sentence in `meta.error` telling the user it is still
//! there. The alternative — tidying away a lecture nobody can get back — is
//! the single worst thing this app could do.
//!
//! [`FLUSH_INTERVAL_SECS`]: crate::capture::FLUSH_INTERVAL_SECS

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::storage::{RecordingRef, Status, Store};

use super::flac::finalize_to_flac;
use super::{MIC_TRACK, SAMPLE_RATE, SYSTEM_TRACK};

/// Smallest byte count that could possibly be a WAV: the 12-byte RIFF/WAVE
/// preamble, a 24-byte PCM `fmt ` chunk, and an 8-byte `data` chunk header.
const MIN_WAV_LEN: u64 = 12 + 24 + 8;

/// What the user is told when a recording's audio file cannot be made
/// readable. Deliberately free of paths, codes, and jargon: the one thing the
/// reader needs to know is that nothing was thrown away.
const UNREPAIRABLE: &str =
    "This recording was interrupted and its audio file is damaged, so Notetaker could not open \
     it. The file has been left exactly as it was — nothing was deleted — in case it can be \
     rescued later.";

/// Where a WAV's declared lengths live, and what they ought to say.
struct Layout {
    /// Byte offset of the `data` chunk's own size field.
    size_field_at: u64,
    /// Byte offset of the first audio byte.
    audio_at: u64,
    /// Audio bytes the header currently claims.
    declared: u64,
    /// Bytes per frame — 2 for the 16-bit mono tracks capture writes.
    block_align: u64,
}

/// Rewrites the RIFF and `data` lengths of `path` to describe the audio the
/// file actually holds, and returns the recovered sample count.
///
/// This is the crash case: a writer that died between flushes leaves a header
/// claiming far fewer bytes than the file contains, so every reader stops
/// early and the rest of the lecture is invisible. It also covers the reverse
/// — a header claiming more than survived — by shrinking the claim to what is
/// really there.
///
/// A file whose header is already right is left completely untouched, not
/// rewritten with identical bytes: recovery runs over every recording at
/// startup, and it has no business changing the modification time of files
/// that were fine.
///
/// # Errors
///
/// A file too short, or too damaged, to be a WAV at all. In that case the file
/// is left exactly as found — this function never truncates and never deletes.
pub fn repair_wav_header(path: &Path) -> Result<u64> {
    let mut file =
        File::open(path).with_context(|| format!("opening {} to repair it", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("measuring {}", path.display()))?
        .len();

    let layout = read_layout(&mut file, len, path)?;

    // Round down to a whole frame: a crash can land mid-sample, and half a
    // sample is not audio. The stray byte stays in the file; it is simply not
    // claimed by the header.
    let available = len - layout.audio_at;
    let usable = (available / layout.block_align) * layout.block_align;

    // WAV's lengths are 32-bit. Nothing this app records comes close, but a
    // corrupt or concatenated file could, and a wrapped length would describe
    // the wrong audio.
    let ceiling = (u32::MAX as u64 / layout.block_align) * layout.block_align;
    let usable = usable.min(ceiling);
    let frames = usable / layout.block_align;

    if layout.declared == usable {
        return Ok(frames);
    }

    log::info!(
        "repairing {}: header claimed {} audio bytes, file holds {usable}",
        path.display(),
        layout.declared
    );

    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("opening {} for repair", path.display()))?;

    // RIFF's ChunkSize covers everything after its own 8-byte preamble.
    let riff_size = (layout.audio_at + usable - 8) as u32;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.seek(SeekFrom::Start(layout.size_field_at))?;
    file.write_all(&(usable as u32).to_le_bytes())?;

    // Push the repair through to the platter: the next thing to go wrong
    // should not undo the recovery of the last thing that went wrong.
    file.sync_all()
        .with_context(|| format!("flushing the repaired header of {}", path.display()))?;

    Ok(frames)
}

/// Walks the RIFF chunk list far enough to find `fmt ` and `data`.
fn read_layout(file: &mut File, len: u64, path: &Path) -> Result<Layout> {
    if len < MIN_WAV_LEN {
        bail!(
            "{} is only {len} bytes — too short to be a wav file at all",
            path.display()
        );
    }

    let mut preamble = [0u8; 12];
    file.read_exact(&mut preamble)
        .with_context(|| format!("reading the header of {}", path.display()))?;
    if &preamble[0..4] != b"RIFF" || &preamble[8..12] != b"WAVE" {
        bail!("{} is not a RIFF/WAVE file", path.display());
    }

    let mut block_align: Option<u64> = None;
    let mut cursor = 12u64;

    while cursor + 8 <= len {
        file.seek(SeekFrom::Start(cursor))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)
            .with_context(|| format!("reading a chunk header in {}", path.display()))?;
        let id = &header[0..4];
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as u64;
        let payload_at = cursor + 8;

        if id == b"fmt " {
            // Channels at +2, bits per sample at +14 of the fmt payload —
            // the same in plain PCM and WAVE_FORMAT_EXTENSIBLE.
            if size < 16 || payload_at + 16 > len {
                bail!("{}: its format chunk is cut short", path.display());
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt)
                .with_context(|| format!("reading the format chunk of {}", path.display()))?;
            let channels = u16::from_le_bytes([fmt[2], fmt[3]]) as u64;
            let bits = u16::from_le_bytes([fmt[14], fmt[15]]) as u64;
            let align = channels * bits.div_ceil(8);
            if align == 0 {
                bail!(
                    "{}: its format chunk describes {channels} channels at {bits} bits",
                    path.display()
                );
            }
            block_align = Some(align);
        } else if id == b"data" {
            let Some(block_align) = block_align else {
                bail!(
                    "{}: its audio starts before it says what format it is in",
                    path.display()
                );
            };
            if payload_at > len {
                bail!("{}: its audio chunk starts past the end", path.display());
            }
            return Ok(Layout {
                size_field_at: cursor + 4,
                audio_at: payload_at,
                declared: size,
                block_align,
            });
        }

        // Chunks are padded to an even length. A bogus size would otherwise
        // run the cursor off the end or, worse, backwards.
        let Some(next) = payload_at.checked_add(size + (size & 1)) else {
            break;
        };
        if next <= cursor {
            break;
        }
        cursor = next;
    }

    bail!(
        "{} has no audio chunk — it is not a wav file we can repair",
        path.display()
    )
}

/// Repairs, finalizes, and requeues every recording left mid-capture, and
/// returns the ids it recovered.
///
/// Run at app start. A recording qualifies when it is still `Recorded` — the
/// status capture leaves behind, and the one the queue has not taken yet — and
/// at least one of its tracks is a `.wav` with no `.flac` beside it. That
/// covers both halves of the problem: a session killed mid-lecture, and a
/// session that stopped cleanly but never got to encode.
///
/// `keep_wav` is the user's setting, passed in rather than read here so that
/// the caller owns the "may I reclaim this space" decision.
///
/// One bad recording never stops the sweep. A recording whose audio cannot be
/// repaired keeps its file, gains a plain-English [`UNREPAIRABLE`] note, and is
/// marked [`Status::Failed`] so it shows up in the UI as something to look at
/// instead of being handed to a pipeline that would only fail on it again.
pub fn recover_orphans(store: &Store, keep_wav: bool) -> Result<Vec<String>> {
    let mut recovered = Vec::new();

    for mut rec in store.scan()? {
        if rec.meta.status != Status::Recorded {
            continue;
        }
        let pending = pending_tracks(&rec.dir);
        if pending.is_empty() {
            continue;
        }

        match recover_one(&mut rec, &pending, keep_wav) {
            Ok(()) => {
                if let Err(e) = store.save_meta(&rec) {
                    // The audio is repaired on disk either way; only the
                    // bookkeeping failed, and the next start will redo it.
                    log::warn!("recovered {} but could not save its meta: {e:#}", rec.meta.id);
                    continue;
                }
                recovered.push(rec.meta.id.clone());
            }
            Err(e) => {
                log::warn!("could not recover {}: {e:#}", rec.dir.display());
                rec.meta.status = Status::Failed;
                rec.meta.error = Some(UNREPAIRABLE.to_string());
                if let Err(e) = store.save_meta(&rec) {
                    log::warn!("could not record the failure of {}: {e:#}", rec.meta.id);
                }
            }
        }
    }

    Ok(recovered)
}

/// The `.wav` tracks of one recording that have no finished `.flac` beside
/// them.
fn pending_tracks(dir: &Path) -> Vec<PathBuf> {
    [MIC_TRACK, SYSTEM_TRACK]
        .iter()
        .map(|stem| dir.join(format!("{stem}.wav")))
        .filter(|wav| wav.is_file() && !wav.with_extension("flac").is_file())
        .collect()
}

/// Repairs and finalizes one recording's tracks, then rewrites its duration.
///
/// A failed *encode* is deliberately not a failed recovery. The repaired WAV
/// is still there and `pipeline::run` reads `.wav` as happily as `.flac`, so
/// the cost of a bad encode is disk space; refusing to requeue the recording
/// over it would cost the transcript.
fn recover_one(rec: &mut RecordingRef, wavs: &[PathBuf], keep_wav: bool) -> Result<()> {
    let mut longest = 0u64;
    for wav in wavs {
        longest = longest.max(repair_wav_header(wav)?);
        if let Err(e) = finalize_to_flac(wav, keep_wav) {
            log::warn!(
                "{} was repaired but would not encode to FLAC, leaving the wav: {e:#}",
                wav.display()
            );
        }
    }

    rec.meta.duration_s = longest as f64 / SAMPLE_RATE as f64;
    rec.meta.status = Status::Recorded;
    // `meta.error` is left alone on purpose. A session that stopped cleanly
    // may have put a true sentence there ("your microphone dropped out"), and
    // the queue clears it on the way to `Queued` anyway.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::track::TrackWriter;
    use crate::capture::FLUSH_INTERVAL_SECS;
    use crate::pipeline::audio::load_mono_16k;
    use crate::storage::{Meta, Mode};
    use chrono::TimeZone;

    fn tone(secs: f64, amp: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * amp
            })
            .collect()
    }

    fn write_wav(path: &Path, frames: &[f32]) {
        let mut track = TrackWriter::create(path).unwrap();
        track.write(frames).unwrap();
        track.finalize().unwrap();
    }

    /// Offset of the `data` chunk's size field. Searched only over the header
    /// region so audio that happens to spell "data" cannot fool it.
    fn data_size_offset(bytes: &[u8]) -> usize {
        bytes[..128]
            .windows(4)
            .position(|w| w == b"data")
            .expect("a wav written by TrackWriter has a data chunk")
            + 4
    }

    /// Rewinds a finished wav's header to claim only `kept` samples — exactly
    /// what a power cut leaves behind, since the header is only rewritten
    /// every `FLUSH_INTERVAL_SECS` while the audio streams out continuously.
    fn understate_header(path: &Path, kept: u64) {
        let mut bytes = std::fs::read(path).unwrap();
        let at = data_size_offset(&bytes);
        let audio = (kept * 2) as u32;
        bytes[at..at + 4].copy_from_slice(&audio.to_le_bytes());
        let riff = (at + 4 - 8) as u32 + audio;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    /// A recording folder on disk in whatever state the test needs.
    fn seed(store: &Store, title: &str, status: Status) -> RecordingRef {
        let created = chrono::Local.with_ymd_and_hms(2026, 7, 27, 9, 0, 0).unwrap();
        let mut rec = store.create_recording(title, Mode::InPerson, created).unwrap();
        rec.meta.status = status;
        store.save_meta(&rec).unwrap();
        rec
    }

    fn meta_on_disk(store: &Store, id: &str) -> Meta {
        store
            .scan()
            .unwrap()
            .into_iter()
            .find(|r| r.meta.id == id)
            .expect("recording must still be on disk")
            .meta
    }

    #[test]
    fn a_crash_between_flushes_gives_back_every_sample_that_reached_disk() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");

        // Longer than a flush interval, so the loss a crash would cause is the
        // real one rather than an artificial nibble.
        let frames = tone(FLUSH_INTERVAL_SECS as f64 + 2.0, 0.5);
        write_wav(&wav, &frames);
        let whole = load_mono_16k(&wav).unwrap();

        let last_flush = FLUSH_INTERVAL_SECS * SAMPLE_RATE as u64;
        understate_header(&wav, last_flush);
        assert_eq!(
            load_mono_16k(&wav).unwrap().len() as u64,
            last_flush,
            "the simulated crash must actually hide the tail, or this proves nothing"
        );

        let recovered = repair_wav_header(&wav).unwrap();

        assert_eq!(recovered, frames.len() as u64);
        assert_eq!(
            load_mono_16k(&wav).unwrap(),
            whole,
            "every sample that reached disk must come back, in order and unchanged"
        );
    }

    #[test]
    fn a_header_claiming_more_than_survived_is_shrunk_to_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        write_wav(&wav, &tone(1.0, 0.5));

        // The header survived the crash but the audio behind it did not.
        let mut bytes = std::fs::read(&wav).unwrap();
        bytes.truncate(bytes.len() - 8000);
        std::fs::write(&wav, &bytes).unwrap();

        let recovered = repair_wav_header(&wav).unwrap();
        assert_eq!(recovered, SAMPLE_RATE as u64 - 4000);
        assert_eq!(load_mono_16k(&wav).unwrap().len(), recovered as usize);
        assert_eq!(
            std::fs::read(&wav).unwrap().len(),
            bytes.len(),
            "repair adjusts the header, it never truncates the file"
        );
    }

    #[test]
    fn a_healthy_wav_is_left_byte_for_byte_alone() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("audio-mic.wav");
        let frames = tone(0.5, 0.5);
        write_wav(&wav, &frames);
        let before = std::fs::read(&wav).unwrap();

        let recovered = repair_wav_header(&wav).unwrap();

        assert_eq!(recovered, frames.len() as u64);
        assert_eq!(
            std::fs::read(&wav).unwrap(),
            before,
            "a file that was already correct must not be rewritten"
        );
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_reported_and_never_touched() {
        let dir = tempfile::tempdir().unwrap();

        let cases: [(&str, Vec<u8>); 3] = [
            ("garbage", b"this was never audio, it is somebody's notes".to_vec()),
            ("stub", b"RIFF".to_vec()),
            // Right preamble, nothing behind it: the shape a half-created file
            // takes, and the one most likely to fool a lazy parser.
            ("headless", {
                let mut v = b"RIFF\x24\x00\x00\x00WAVE".to_vec();
                v.extend(std::iter::repeat_n(0u8, 40));
                v
            }),
        ];

        for (name, bytes) in cases {
            let path = dir.path().join(format!("{name}.wav"));
            std::fs::write(&path, &bytes).unwrap();

            assert!(
                repair_wav_header(&path).is_err(),
                "{name}: a file we cannot parse must be reported, not guessed at"
            );
            assert!(path.exists(), "{name}: nothing here may delete a file");
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "{name}: an unrepairable file must be left byte for byte as found"
            );
        }
    }

    #[test]
    fn recovery_touches_only_the_recording_that_crashed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        // Already finished: a verified FLAC, no wav, and marked Ready.
        let mut healthy = seed(&store, "Finished lecture", Status::Recorded);
        write_wav(&healthy.dir.join("audio-mic.wav"), &tone(0.5, 0.4));
        finalize_to_flac(&healthy.dir.join("audio-mic.wav"), false).unwrap();
        healthy.meta.status = Status::Ready;
        healthy.meta.duration_s = 0.5;
        store.save_meta(&healthy).unwrap();
        let healthy_flac = std::fs::read(healthy.dir.join("audio-mic.flac")).unwrap();

        // Killed mid-lecture: a wav whose header understates it, no flac.
        let crashed = seed(&store, "Interrupted lecture", Status::Recorded);
        let crashed_wav = crashed.dir.join("audio-mic.wav");
        let frames = tone(FLUSH_INTERVAL_SECS as f64 + 1.0, 0.5);
        write_wav(&crashed_wav, &frames);
        understate_header(&crashed_wav, FLUSH_INTERVAL_SECS * SAMPLE_RATE as u64);

        let ids = recover_orphans(&store, false).unwrap();

        assert_eq!(
            ids,
            vec![crashed.meta.id.clone()],
            "only the interrupted recording was recovered"
        );

        let meta = meta_on_disk(&store, &crashed.meta.id);
        assert_eq!(meta.status, Status::Recorded, "the queue must pick this up");
        assert!(
            (meta.duration_s - (FLUSH_INTERVAL_SECS as f64 + 1.0)).abs() < 1e-9,
            "duration must be the recovered length, got {}",
            meta.duration_s
        );
        assert!(!crashed_wav.exists(), "keep_wav = false reclaims the space");
        assert_eq!(
            load_mono_16k(&crashed.dir.join("audio-mic.flac")).unwrap().len(),
            frames.len(),
            "the recovered audio must survive the encode intact"
        );

        let healthy_meta = meta_on_disk(&store, &healthy.meta.id);
        assert_eq!(healthy_meta.status, Status::Ready, "a finished recording is not re-run");
        assert_eq!(healthy_meta.duration_s, 0.5);
        assert_eq!(
            std::fs::read(healthy.dir.join("audio-mic.flac")).unwrap(),
            healthy_flac,
            "a finished recording's audio must not be rewritten"
        );
    }

    #[test]
    fn an_unrepairable_recording_is_kept_on_disk_with_an_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let broken = seed(&store, "Interrupted lecture", Status::Recorded);
        let wav = broken.dir.join("audio-mic.wav");
        std::fs::write(&wav, b"whatever this is, it is the only copy").unwrap();

        let ids = recover_orphans(&store, false).unwrap();

        assert!(ids.is_empty(), "nothing was actually recovered");
        assert!(wav.exists(), "the only copy must never be deleted");
        assert_eq!(
            std::fs::read(&wav).unwrap(),
            b"whatever this is, it is the only copy",
            "an unrepairable file must be left exactly as found"
        );

        let meta = meta_on_disk(&store, &broken.meta.id);
        assert_eq!(
            meta.status,
            Status::Failed,
            "a recording we could not repair must not be fed to the pipeline"
        );
        let message = meta.error.expect("the user must be told what happened");
        assert!(message.contains("nothing was deleted"), "{message}");
        assert!(
            !message.contains("RIFF") && !message.contains("Err") && !message.contains(".wav"),
            "the message must read as plain English, not a log line: {message}"
        );
    }

    #[test]
    fn both_tracks_of_an_interrupted_meeting_are_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let rec = seed(&store, "Interrupted standup", Status::Recorded);
        let mic = rec.dir.join("audio-mic.wav");
        let system = rec.dir.join("audio-system.wav");
        write_wav(&mic, &tone(1.0, 0.5));
        write_wav(&system, &tone(2.0, 0.3));
        understate_header(&mic, 0);
        understate_header(&system, 0);

        let ids = recover_orphans(&store, true).unwrap();

        assert_eq!(ids, vec![rec.meta.id.clone()]);
        assert_eq!(
            load_mono_16k(&rec.dir.join("audio-mic.flac")).unwrap().len(),
            SAMPLE_RATE as usize
        );
        assert_eq!(
            load_mono_16k(&rec.dir.join("audio-system.flac")).unwrap().len(),
            2 * SAMPLE_RATE as usize
        );
        assert!(mic.exists() && system.exists(), "keep_wav = true keeps both");
        assert!(
            (meta_on_disk(&store, &rec.meta.id).duration_s - 2.0).abs() < 1e-9,
            "duration is the longer track — one can die before the other"
        );
    }

    #[test]
    fn a_recording_already_moving_through_the_queue_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        for status in [Status::Queued, Status::Processing, Status::Ready, Status::Failed] {
            let rec = seed(&store, &format!("{status:?}"), status);
            write_wav(&rec.dir.join("audio-mic.wav"), &tone(0.3, 0.4));
        }

        assert!(
            recover_orphans(&store, false).unwrap().is_empty(),
            "recovery only claims recordings capture has finished with"
        );
        for r in store.scan().unwrap() {
            assert!(
                r.dir.join("audio-mic.wav").exists(),
                "{:?}: an in-flight recording's audio must not be touched",
                r.meta.status
            );
        }
    }

    #[test]
    fn a_store_with_nothing_to_recover_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        assert!(recover_orphans(&store, false).unwrap().is_empty());

        // A recording whose FLAC already exists is finished, wav or no wav.
        let rec = seed(&store, "Finished", Status::Recorded);
        write_wav(&rec.dir.join("audio-mic.wav"), &tone(0.3, 0.4));
        finalize_to_flac(&rec.dir.join("audio-mic.wav"), true).unwrap();
        assert!(recover_orphans(&store, false).unwrap().is_empty());
        assert!(
            rec.dir.join("audio-mic.wav").exists(),
            "a second pass must not re-finalize and delete a kept wav"
        );
        assert_eq!(meta_on_disk(&store, &rec.meta.id).duration_s, 0.0);
    }
}
