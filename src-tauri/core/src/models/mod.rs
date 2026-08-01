//! Local model registry, hardware-tier detection, and resumable
//! checksum-verified downloads.
//!
//! On first run the app needs to fetch a handful of large speech and
//! diarization models onto the user's Mac. This module hardcodes exactly
//! which URLs may ever be fetched (see [`registry`] — that file IS the
//! allowlist), picks which of them are needed for the detected hardware
//! tier, and downloads+verifies them with resume support so a crash mid
//! download never leaves a corrupt "complete" model on disk.

pub mod existing;
pub mod registry;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use sha2::{Digest, Sha256};

/// A single downloadable model artifact: the allowlisted URL, its expected
/// sha256 (hex, lowercase; empty means "not yet verified, skip the check"),
/// and the filename it lands under inside the models directory.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub dest: &'static str,
    /// What this is, in words a user reads. `name` is an identifier and shows
    /// up in settings files; this is the only string that should reach a
    /// screen.
    pub label: &'static str,
    /// Download size in bytes, so the app can say what it is about to cost
    /// someone before they agree to it.
    ///
    /// Every figure here was read from the `content-length` of the real URL
    /// above on 2026-07-30, not estimated. It is used for display only —
    /// nothing verifies a download against it, because the sha256 already
    /// does that job properly. A number that drifts makes a label slightly
    /// wrong; it can never let a bad file through.
    pub bytes: u64,
}

/// Hardware capability tier, used to pick which models to fetch.
///
/// The three original names are a **settings-file contract** —
/// `Settings::tier_override` stores one of these strings, so renaming one
/// would silently invalidate a user's saved choice. `CpuBig` was added for
/// Windows and Intel desktops; the others are untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    AppleSiliconBig,
    AppleSiliconSmall,
    /// A desktop-class CPU with room to run the large model, just slowly.
    CpuBig,
    CpuSmall,
}

/// RAM floor, in GB, for Apple Silicon to get the large model.
const APPLE_BIG_RAM_GB: u64 = 16;
/// RAM floor, in GB, for a non-Apple machine to get the large model.
const CPU_BIG_RAM_GB: u64 = 32;
/// Core floor for a non-Apple machine to get the large model.
const CPU_BIG_CORES: usize = 8;

/// Which models this machine should fetch.
///
/// Apple Silicon with >= 16 GB gets the large model on the GPU; under that,
/// the small one. For everything else the question is different: there is no
/// Metal path, so the large model runs on CPU cores at roughly real time or
/// worse. That is a bad trade for a laptop and a fine one for a desktop —
/// **transcription here is idle-time background work with no deadline**, so a
/// machine with the cores and memory to spare gets the better transcript and
/// takes longer over it. `CPU_BIG_CORES` and `CPU_BIG_RAM_GB` together are the
/// "this is a desktop, not a thin laptop" test; a 32 GB dual-core would be an
/// odd machine and it stays on the small model.
///
/// This is a *default*, not a verdict — `Settings::tier_override` lets the
/// user move either way, which matters because we cannot measure how patient
/// they are.
pub fn detect_tier(total_ram_gb: u64, cpu_cores: usize, is_apple_silicon: bool) -> Tier {
    if is_apple_silicon {
        if total_ram_gb >= APPLE_BIG_RAM_GB {
            Tier::AppleSiliconBig
        } else {
            Tier::AppleSiliconSmall
        }
    } else if total_ram_gb >= CPU_BIG_RAM_GB && cpu_cores >= CPU_BIG_CORES {
        Tier::CpuBig
    } else {
        Tier::CpuSmall
    }
}

/// Downloads and verifies [`ModelSpec`]s into `models_dir`, resuming partial
/// downloads via HTTP `Range` and verifying sha256 before a file is treated
/// as complete.
pub struct Downloader {
    pub models_dir: PathBuf,
}

impl Downloader {
    /// Ensures `spec` is present (fully downloaded and checksum-verified)
    /// under `models_dir`. Downloads to `<dest>.part`, resuming from where a
    /// previous attempt left off if a partial file exists, then verifies the
    /// sha256 and renames into place. A checksum mismatch deletes the
    /// partial and returns an error whose message mentions "checksum".
    /// `progress` is called with `(downloaded_bytes, total_bytes_hint)` as
    /// data streams in.
    pub fn ensure<F: FnMut(u64, u64)>(
        &self,
        spec: &ModelSpec,
        mut progress: F,
    ) -> anyhow::Result<PathBuf> {
        fs::create_dir_all(&self.models_dir)
            .with_context(|| format!("creating models dir {}", self.models_dir.display()))?;

        let dest_path = self.models_dir.join(spec.dest);
        if dest_path.exists() {
            // Already downloaded and verified by a previous ensure() call.
            return Ok(dest_path);
        }

        let part_path = self.models_dir.join(format!("{}.part", spec.dest));
        let mut resume_from = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

        let mut request = ureq::get(spec.url);
        if resume_from > 0 {
            request = request.header("Range", format!("bytes={resume_from}-"));
        }

        let response = request
            .call()
            .with_context(|| format!("requesting {}", spec.url))?;

        // Only treat this as a resumed transfer if the server actually
        // honored the Range request (206 Partial Content). Otherwise fall
        // back to a full download, overwriting the stale partial.
        let resumed = resume_from > 0 && response.status().as_u16() == 206;
        if !resumed {
            resume_from = 0;
        }

        let body = response.into_body();
        let remaining_hint = body.content_length();
        let total_hint = remaining_hint.map(|n| n + resume_from);
        let mut reader = body.into_reader();

        let mut file = if resumed {
            fs::OpenOptions::new().append(true).open(&part_path)
        } else {
            fs::File::create(&part_path)
        }
        .with_context(|| format!("opening {}", part_path.display()))?;

        let mut downloaded = resume_from;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).context("reading download body")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context("writing partial file")?;
            downloaded += n as u64;
            progress(downloaded, total_hint.unwrap_or(downloaded));
        }
        file.flush().context("flushing partial file")?;
        drop(file);

        if !spec.sha256.is_empty() {
            let actual = sha256_hex_of_file(&part_path)?;
            if actual != spec.sha256 {
                let _ = fs::remove_file(&part_path);
                anyhow::bail!(
                    "checksum mismatch for {}: expected {}, got {actual}",
                    spec.name,
                    spec.sha256
                );
            }
        }

        fs::rename(&part_path, &dest_path)
            .with_context(|| format!("finalizing {}", dest_path.display()))?;
        Ok(dest_path)
    }
}

fn sha256_hex_of_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

/// What the segmentation model is called once it has been unpacked.
///
/// A name we choose, not one taken from the archive — see
/// [`ensure_segmentation_unpacked`].
pub const SEGMENTATION_ONNX: &str = "pyannote-segmentation-3-0.onnx";

/// The name of the single file worth taking out of the segmentation archive.
const SEGMENTATION_MEMBER: &str = "model.onnx";

/// Unpacks the segmentation model and returns the path to it.
///
/// The sherpa-onnx project ships `segmentation-3.0` as a `.tar.bz2` holding the
/// `.onnx` alongside a licence and the export scripts, but [`SherpaDiarizer`]
/// needs a path to the `.onnx` itself. Nothing did this unpacking, which is why
/// the models could be downloaded in full and diarization would still fail to
/// load — the archive was the last unwired step between "downloaded" and
/// "usable".
///
/// [`SherpaDiarizer`]: crate::pipeline::diarize::SherpaDiarizer
///
/// **The archive never names the destination.** Entry paths inside a tarball
/// are attacker-controlled in the general case (`../../` escapes the directory
/// being extracted into), so this takes the *one* member whose file name is
/// `model.onnx` and writes it to a path this function chose. Nothing else in
/// the archive is written anywhere. That removes the traversal question
/// entirely rather than answering it with a check that has to be right.
///
/// Idempotent: an already-unpacked model is returned without re-reading the
/// archive, and a crash midway leaves only a `.part` file behind.
pub fn ensure_segmentation_unpacked(models_dir: &Path) -> anyhow::Result<PathBuf> {
    let dest = models_dir.join(SEGMENTATION_ONNX);
    if dest.exists() {
        return Ok(dest);
    }

    let archive_path = models_dir.join(registry::DIARIZATION_SEGMENTATION.dest);
    let archive = fs::File::open(&archive_path).with_context(|| {
        format!(
            "opening the speaker-segmentation archive {}",
            archive_path.display()
        )
    })?;

    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(archive));
    let entries = tar
        .entries()
        .with_context(|| format!("reading {}", archive_path.display()))?;

    let part = models_dir.join(format!("{SEGMENTATION_ONNX}.part"));
    for entry in entries {
        let mut entry = entry.with_context(|| format!("reading {}", archive_path.display()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let is_the_model = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == SEGMENTATION_MEMBER))
            .unwrap_or(false);
        if !is_the_model {
            continue;
        }

        let mut out =
            fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("unpacking {SEGMENTATION_MEMBER} to {}", part.display()))?;
        out.sync_all().ok();
        drop(out);

        fs::rename(&part, &dest).with_context(|| format!("finalizing {}", dest.display()))?;
        return Ok(dest);
    }

    anyhow::bail!(
        "{} does not contain a {SEGMENTATION_MEMBER}, so speaker separation cannot start. \
         Deleting it and downloading the models again should fix it.",
        archive_path.display()
    )
}

/// Lowercase-hex-encodes bytes without pulling in a `hex` crate dependency.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn leak_url(path: &str, server: &MockServer) -> &'static str {
        Box::leak(server.url(path).into_boxed_str())
    }

    // --- unpacking the segmentation archive ---------------------------------

    /// Writes a `.tar.bz2` under the name the registry expects, containing one
    /// file at `inner_path` with `body`.
    fn write_archive(models_dir: &Path, inner_path: &str, body: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        // The name is written into the raw header rather than through
        // `set_path`, which refuses to produce a path containing `..`. A
        // hostile archive is not built with the polite API either, and an
        // escape this function will not construct is an escape it cannot test
        // against.
        let name = header.as_old_mut();
        let bytes = inner_path.as_bytes();
        name.name[..bytes.len()].copy_from_slice(bytes);
        header.set_cksum();

        let mut tar = tar::Builder::new(Vec::new());
        tar.append(&header, body).unwrap();
        let tar_bytes = tar.into_inner().unwrap();

        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        bz.write_all(&tar_bytes).unwrap();
        let bz_bytes = bz.finish().unwrap();

        fs::write(
            models_dir.join(registry::DIARIZATION_SEGMENTATION.dest),
            bz_bytes,
        )
        .unwrap();
    }

    #[test]
    fn unpacking_lifts_the_onnx_out_of_the_release_tarball() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            "sherpa-onnx-pyannote-segmentation-3-0/model.onnx",
            b"onnx bytes",
        );

        let path = ensure_segmentation_unpacked(dir.path()).unwrap();

        assert_eq!(path, dir.path().join(SEGMENTATION_ONNX));
        assert_eq!(fs::read(&path).unwrap(), b"onnx bytes");
        assert!(
            !dir.path()
                .join(format!("{SEGMENTATION_ONNX}.part"))
                .exists(),
            "the partial file must not survive a successful unpack"
        );
    }

    #[test]
    fn the_archive_never_chooses_where_a_file_lands() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let models_dir = dir.path().join("models");
        fs::create_dir_all(&models_dir).unwrap();

        // A member that tries to escape into a sibling directory.
        write_archive(&models_dir, "../outside/model.onnx", b"escaped");

        let path = ensure_segmentation_unpacked(&models_dir).unwrap();

        assert_eq!(
            path,
            models_dir.join(SEGMENTATION_ONNX),
            "the destination must be ours, not the archive's"
        );
        assert_eq!(fs::read(&path).unwrap(), b"escaped");
        assert!(
            !outside.join("model.onnx").exists(),
            "nothing may be written outside the models directory"
        );
    }

    #[test]
    fn an_already_unpacked_model_does_not_need_the_archive_again() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SEGMENTATION_ONNX), b"already here").unwrap();
        // No archive on disk at all.

        let path = ensure_segmentation_unpacked(dir.path()).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"already here");
    }

    #[test]
    fn a_missing_archive_says_which_file_it_wanted() {
        let dir = tempfile::tempdir().unwrap();

        let err = ensure_segmentation_unpacked(dir.path()).unwrap_err();

        let message = format!("{err:#}");
        assert!(
            message.contains(registry::DIARIZATION_SEGMENTATION.dest),
            "unhelpful message: {message}"
        );
    }

    #[test]
    fn an_archive_without_the_model_says_so_rather_than_half_succeeding() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(dir.path(), "sherpa-onnx-pyannote/LICENSE", b"MIT");

        let err = ensure_segmentation_unpacked(dir.path()).unwrap_err();

        assert!(
            format!("{err:#}").contains("model.onnx"),
            "unhelpful message: {err:#}"
        );
        assert!(!dir.path().join(SEGMENTATION_ONNX).exists());
    }

    fn body_1kb() -> Vec<u8> {
        (0..1024u32).map(|i| (i % 256) as u8).collect()
    }

    // --- detect_tier ---------------------------------------------------

    /// Core count is irrelevant on Apple Silicon — the large model runs on the
    /// GPU there, so RAM is the only question. Passed as 4 (a low count) in
    /// these two to prove it does not drag an Apple machine down a tier.
    #[test]
    fn detect_tier_apple_silicon_16gb_is_big() {
        assert_eq!(detect_tier(16, 4, true), Tier::AppleSiliconBig);
    }

    #[test]
    fn detect_tier_apple_silicon_8gb_is_small() {
        assert_eq!(detect_tier(8, 4, true), Tier::AppleSiliconSmall);
    }

    #[test]
    fn detect_tier_boundary_just_under_16gb_is_still_small() {
        assert_eq!(detect_tier(15, 4, true), Tier::AppleSiliconSmall);
    }

    /// A desktop-class Windows/Intel box gets the large model, run on CPU.
    /// This is a deliberate change from Plan A/B, where every non-Apple
    /// machine was `CpuSmall` regardless of how capable it was.
    #[test]
    fn detect_tier_non_apple_desktop_class_is_cpu_big() {
        assert_eq!(detect_tier(32, 8, false), Tier::CpuBig);
    }

    /// Both floors must be met. A machine with plenty of RAM but few cores
    /// would spend hours per recording on the large model, so it stays small.
    #[test]
    fn detect_tier_non_apple_ram_without_cores_stays_small() {
        assert_eq!(detect_tier(64, 4, false), Tier::CpuSmall);
    }

    /// ...and the mirror case: many cores but not enough memory.
    #[test]
    fn detect_tier_non_apple_cores_without_ram_stays_small() {
        assert_eq!(detect_tier(16, 16, false), Tier::CpuSmall);
    }

    #[test]
    fn detect_tier_typical_laptop_is_cpu_small() {
        assert_eq!(detect_tier(16, 8, false), Tier::CpuSmall);
    }

    // --- Downloader::ensure --------------------------------------------

    #[test]
    fn full_download_writes_file_and_passes_checksum() {
        let server = MockServer::start();
        let body = body_1kb();
        let expected_sha256 = hex_encode(Sha256::digest(&body).as_slice());
        let mock = server.mock(|when, then| {
            when.method(GET).path("/model.bin");
            then.status(200).body(&body);
        });

        let dir = tempfile::tempdir().unwrap();
        let downloader = Downloader {
            models_dir: dir.path().to_path_buf(),
        };
        let spec = ModelSpec {
            name: "test-model",
            url: leak_url("/model.bin", &server),
            sha256: Box::leak(expected_sha256.into_boxed_str()),
            dest: "model.bin",
            label: "A model, for a test",
            bytes: 0,
        };

        let result = downloader.ensure(&spec, |_, _| {}).unwrap();

        mock.assert();
        assert_eq!(result, dir.path().join("model.bin"));
        assert_eq!(fs::read(&result).unwrap(), body);
        assert!(!dir.path().join("model.bin.part").exists());
    }

    #[test]
    fn corrupted_body_returns_checksum_error_and_removes_partial() {
        let server = MockServer::start();
        let body = body_1kb();
        // Deliberately wrong hash, as if the bytes were corrupted in transit.
        let wrong_sha256 = "0".repeat(64);
        let mock = server.mock(|when, then| {
            when.method(GET).path("/model.bin");
            then.status(200).body(&body);
        });

        let dir = tempfile::tempdir().unwrap();
        let downloader = Downloader {
            models_dir: dir.path().to_path_buf(),
        };
        let spec = ModelSpec {
            name: "test-model",
            url: leak_url("/model.bin", &server),
            sha256: Box::leak(wrong_sha256.into_boxed_str()),
            dest: "model.bin",
            label: "A model, for a test",
            bytes: 0,
        };

        let err = downloader.ensure(&spec, |_, _| {}).unwrap_err();

        mock.assert();
        assert!(
            err.to_string().to_lowercase().contains("checksum"),
            "error should mention checksum, got: {err}"
        );
        assert!(!dir.path().join("model.bin").exists());
        assert!(!dir.path().join("model.bin.part").exists());
    }

    #[test]
    fn resumes_partial_download_with_range_header() {
        let server = MockServer::start();
        let body = body_1kb();
        let split_at = 400usize;
        let (already_have, remaining) = body.split_at(split_at);
        let expected_sha256 = hex_encode(Sha256::digest(&body).as_slice());

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/model.bin")
                .header("Range", format!("bytes={split_at}-"));
            then.status(206).body(remaining);
        });

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("model.bin.part"), already_have).unwrap();

        let downloader = Downloader {
            models_dir: dir.path().to_path_buf(),
        };
        let spec = ModelSpec {
            name: "test-model",
            url: leak_url("/model.bin", &server),
            sha256: Box::leak(expected_sha256.into_boxed_str()),
            dest: "model.bin",
            label: "A model, for a test",
            bytes: 0,
        };

        let result = downloader.ensure(&spec, |_, _| {}).unwrap();

        mock.assert();
        assert_eq!(fs::read(&result).unwrap(), body);
    }

    #[test]
    fn empty_expected_hash_skips_verification() {
        let server = MockServer::start();
        let body = body_1kb();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/model.bin");
            then.status(200).body(&body);
        });

        let dir = tempfile::tempdir().unwrap();
        let downloader = Downloader {
            models_dir: dir.path().to_path_buf(),
        };
        let spec = ModelSpec {
            name: "test-model",
            url: leak_url("/model.bin", &server),
            sha256: "", // deliberate: unverified TODO entries skip the check
            dest: "model.bin",
            label: "A model, for a test",
            bytes: 0,
        };

        let result = downloader.ensure(&spec, |_, _| {}).unwrap();

        mock.assert();
        assert_eq!(fs::read(&result).unwrap(), body);
    }

    #[test]
    fn ensure_is_idempotent_and_does_not_redownload_a_complete_file() {
        let server = MockServer::start();
        let body = body_1kb();
        let expected_sha256 = hex_encode(Sha256::digest(&body).as_slice());
        let mock = server.mock(|when, then| {
            when.method(GET).path("/model.bin");
            then.status(200).body(&body);
        });

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("model.bin"), &body).unwrap();

        let downloader = Downloader {
            models_dir: dir.path().to_path_buf(),
        };
        let spec = ModelSpec {
            name: "test-model",
            url: leak_url("/model.bin", &server),
            sha256: Box::leak(expected_sha256.into_boxed_str()),
            dest: "model.bin",
            label: "A model, for a test",
            bytes: 0,
        };

        let result = downloader.ensure(&spec, |_, _| {}).unwrap();

        assert_eq!(
            mock.calls(),
            0,
            "should not hit the network for an already-complete file"
        );
        assert_eq!(fs::read(&result).unwrap(), body);
    }
}
