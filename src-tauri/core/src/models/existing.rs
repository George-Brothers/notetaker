//! Finds and safely adopts speech models the user already downloaded.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{sha256_hex_of_file, ModelSpec};

/// A candidate model found outside Notetaker's own models directory.
#[derive(Debug, Clone)]
pub struct Found {
    pub spec: &'static ModelSpec,
    pub path: PathBuf,
}

/// Directories where the supported model tools normally keep their downloads.
///
/// The scan is deliberately limited to these small, recognizable roots. A
/// recursive walk of somebody's home directory while opening a checklist is a
/// freeze disguised as a convenience feature.
pub fn search_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let mut roots = Vec::new();

    if let Some(home) = home {
        roots.push(home.join(".cache").join("huggingface").join("hub"));
        roots.push(home.join(".cache").join("whisper"));
        roots.push(home.join("Downloads"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local_app_data).join("whisper"));
    }
    roots
}

/// Finds size-compatible files under the known model roots.
///
/// Hashing a model can take a noticeable amount of time; length is a free
/// rejection test, so no wrong-size file reaches [`adopt`]'s checksum pass.
pub fn candidates(roots: &[PathBuf], spec: &'static ModelSpec) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        find_size_matches(root, spec.bytes, 3, &mut found);
    }
    found
}

fn find_size_matches(dir: &Path, bytes: u64, levels_left: u8, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            if metadata.len() == bytes {
                found.push(path);
            }
        } else if metadata.is_dir() && levels_left > 0 {
            find_size_matches(&path, bytes, levels_left - 1, found);
        }
    }
}

/// Verifies a candidate and copies it into Notetaker's model directory.
///
/// A mismatch is ordinary — another application can have a same-size model
/// with a different revision — so it returns `Ok(false)`. The source is never
/// moved, linked, or modified.
pub fn adopt(candidate: &Path, spec: &ModelSpec, models_dir: &Path) -> Result<bool> {
    let Ok(metadata) = fs::metadata(candidate) else {
        return Ok(false);
    };
    if !metadata.is_file() || metadata.len() != spec.bytes || spec.sha256.is_empty() {
        return Ok(false);
    }
    if sha256_hex_of_file(candidate)? != spec.sha256 {
        return Ok(false);
    }

    fs::create_dir_all(models_dir)
        .with_context(|| format!("creating models directory {}", models_dir.display()))?;
    let dest = models_dir.join(spec.dest);
    if dest.exists() {
        return Ok(true);
    }
    let part = models_dir.join(format!("{}.adopting", spec.dest));
    fs::copy(candidate, &part)
        .with_context(|| format!("copying existing model {}", candidate.display()))?;
    fs::rename(&part, &dest).with_context(|| format!("finalizing {}", dest.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelSpec;

    const KNOWN_BYTES: &[u8] = b"Notetaker model fixture";
    const FIXTURE: ModelSpec = ModelSpec {
        name: "fixture",
        url: "https://example.invalid/fixture",
        sha256: "e1ed6da56ff74b85de804c4de61a5c28eafcb00e134b4485d6cbd7806a2a08d3",
        dest: "fixture.onnx",
        label: "Fixture",
        bytes: KNOWN_BYTES.len() as u64,
    };

    #[test]
    fn only_files_whose_size_matches_are_candidates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("wrong-size.onnx"), b"wrong size").unwrap();

        assert!(candidates(&[dir.path().to_path_buf()], &FIXTURE).is_empty());
    }

    #[test]
    fn a_candidate_whose_hash_matches_is_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let source = dir.path().join("found.onnx");
        std::fs::write(&source, KNOWN_BYTES).unwrap();

        assert!(adopt(&source, &FIXTURE, models.path()).unwrap());
        assert_eq!(
            std::fs::read(models.path().join(FIXTURE.dest)).unwrap(),
            KNOWN_BYTES
        );
    }

    #[test]
    fn a_candidate_whose_hash_does_not_match_is_refused_without_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let source = dir.path().join("impostor.onnx");
        std::fs::write(&source, b"Impostor model fixture!").unwrap();

        assert!(!adopt(&source, &FIXTURE, models.path()).unwrap());
        assert!(!models.path().join(FIXTURE.dest).exists());
    }

    #[test]
    fn adopting_never_moves_or_deletes_the_users_own_copy() {
        let dir = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        let source = dir.path().join("theirs.onnx");
        std::fs::write(&source, KNOWN_BYTES).unwrap();

        assert!(adopt(&source, &FIXTURE, models.path()).unwrap());
        assert_eq!(std::fs::read(&source).unwrap(), KNOWN_BYTES);
    }

    #[test]
    fn search_roots_cover_the_places_models_actually_live() {
        let all = search_roots()
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(all.contains("huggingface"), "roots: {all}");
        assert!(all.contains("whisper"), "roots: {all}");
    }
}
