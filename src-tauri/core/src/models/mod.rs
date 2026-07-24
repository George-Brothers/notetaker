//! Local model registry, hardware-tier detection, and resumable
//! checksum-verified downloads.
//!
//! On first run the app needs to fetch a handful of large speech and
//! diarization models onto the user's Mac. This module hardcodes exactly
//! which URLs may ever be fetched (see [`registry`] — that file IS the
//! allowlist), picks which of them are needed for the detected hardware
//! tier, and downloads+verifies them with resume support so a crash mid
//! download never leaves a corrupt "complete" model on disk.

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
}

/// Hardware capability tier, used to pick which models to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    AppleSiliconBig,
    AppleSiliconSmall,
    CpuSmall,
}

/// Apple Silicon with >=16GB RAM gets the big tier; Apple Silicon under that
/// gets the small tier; anything else (Intel, other CPUs) gets the CPU tier.
pub fn detect_tier(total_ram_gb: u64, is_apple_silicon: bool) -> Tier {
    if is_apple_silicon {
        if total_ram_gb >= 16 {
            Tier::AppleSiliconBig
        } else {
            Tier::AppleSiliconSmall
        }
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

/// Lowercase-hex-encodes bytes without pulling in a `hex` crate dependency.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
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

    fn body_1kb() -> Vec<u8> {
        (0..1024u32).map(|i| (i % 256) as u8).collect()
    }

    // --- detect_tier ---------------------------------------------------

    #[test]
    fn detect_tier_apple_silicon_16gb_is_big() {
        assert_eq!(detect_tier(16, true), Tier::AppleSiliconBig);
    }

    #[test]
    fn detect_tier_apple_silicon_8gb_is_small() {
        assert_eq!(detect_tier(8, true), Tier::AppleSiliconSmall);
    }

    #[test]
    fn detect_tier_non_apple_silicon_32gb_is_cpu_small() {
        assert_eq!(detect_tier(32, false), Tier::CpuSmall);
    }

    #[test]
    fn detect_tier_boundary_just_under_16gb_is_still_small() {
        assert_eq!(detect_tier(15, true), Tier::AppleSiliconSmall);
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
        };

        let result = downloader.ensure(&spec, |_, _| {}).unwrap();

        assert_eq!(mock.calls(), 0, "should not hit the network for an already-complete file");
        assert_eq!(fs::read(&result).unwrap(), body);
    }
}
