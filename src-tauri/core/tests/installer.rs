//! What the Windows installer has to carry — checked from Linux, where the
//! installer cannot be built.
//!
//! `sherpa-rs` does not build ONNX Runtime. Its build script downloads a
//! prebuilt copy and then copies the *shared* libraries loose into
//! `target/<profile>/`, which is why a binary run from the build folder works.
//! Tauri's bundler knows nothing about them: without a `resources` entry,
//! `tauri build` produces an installer holding `Notetaker.exe` and nothing
//! else, and the installed app dies on launch with a missing-DLL box.
//!
//! That is the same three files that have now broken this project in three
//! separate places — at link time in CI, at load time in CI, and here. So the
//! rule is pinned by a test rather than by remembering: **every shared library
//! that lands next to our binaries must be named by a pattern in
//! `tauri.windows.conf.json`.** If sherpa ships a fourth one, this fails on
//! Linux in seconds instead of on a PC after an install.
//!
//! The check is one-directional on purpose. It reads the libraries that are
//! actually on disk in this build and asks whether the config covers them; it
//! does not try to predict what Windows will produce. Names are translated to
//! their Windows spelling (`libonnxruntime.so` -> `onnxruntime.dll`), which is
//! the identity mapping when this same test runs on Windows in CI.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// `target/<profile>/` — found from the test binary rather than guessed, so it
/// is right under `--release`, under a custom `CARGO_TARGET_DIR`, and in CI.
///
/// An integration test binary lives at `target/<profile>/deps/<name>-<hash>`.
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    exe.parent()
        .and_then(Path::parent)
        .expect("the test binary is two levels below the profile directory")
        .to_path_buf()
}

/// The Windows file name for a shared library, whatever platform built it.
///
/// `libonnxruntime.so` and `libonnxruntime.dylib` are both `onnxruntime.dll`;
/// `onnxruntime.dll` is itself.
fn windows_dll_name(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    format!("{}.dll", stem.strip_prefix("lib").unwrap_or(stem))
}

/// Every shared library sitting in the profile directory, in its Windows
/// spelling, minus the ones this workspace produces itself.
///
/// The app crate is built as a `cdylib` as well as a binary, so `notetaker_lib`
/// appears here on Windows. It is our own output and the installer has no use
/// for it — everything else is a third-party runtime the app cannot start
/// without.
fn shared_libraries_beside_our_binaries() -> BTreeSet<String> {
    let dir = profile_dir();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading the build directory {}: {e}", dir.display()));

    let mut found = BTreeSet::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_shared = name.ends_with(".so") || name.ends_with(".dylib") || name.ends_with(".dll");
        if !is_shared {
            continue;
        }
        let dll = windows_dll_name(name);
        if dll.starts_with("notetaker") {
            continue;
        }
        found.insert(dll);
    }
    found
}

/// The `bundle.resources` keys of the Windows-only Tauri config.
fn windows_resource_patterns() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tauri.windows.conf.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading the Windows bundle config at {}: {e}",
            path.display()
        )
    });
    let config: serde_json::Value =
        serde_json::from_str(&text).expect("tauri.windows.conf.json is not valid JSON");

    config["bundle"]["resources"]
        .as_object()
        .expect("bundle.resources must be a map of source pattern -> destination")
        .keys()
        .cloned()
        .collect()
}

/// Does a `*`-wildcard pattern match this name? The same shape of glob Tauri
/// hands to the `glob` crate, restricted to the one metacharacter we use.
fn matches(pattern: &str, name: &str) -> bool {
    let mut segments = pattern.split('*');
    let first = segments.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let tail: Vec<&str> = segments.collect();
    let Some((last, middle)) = tail.split_last() else {
        // No `*` at all: the pattern was a literal.
        return rest.is_empty();
    };
    for segment in middle {
        match rest.find(segment) {
            Some(at) => rest = &rest[at + segment.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// The names a set of patterns fails to cover. Split out from the test so the
/// matcher can be shown to fail as well as to pass.
fn uncovered(libraries: &BTreeSet<String>, patterns: &[String]) -> Vec<String> {
    libraries
        .iter()
        .filter(|lib| {
            !patterns.iter().any(|pattern| {
                let file_pattern = pattern.rsplit('/').next().unwrap_or(pattern);
                matches(file_pattern, lib)
            })
        })
        .cloned()
        .collect()
}

#[test]
fn the_windows_installer_carries_every_shared_library_the_app_needs() {
    let libraries = shared_libraries_beside_our_binaries();
    assert!(
        !libraries.is_empty(),
        "no shared libraries were found in {} — this test would pass vacuously, \
         which is worse than failing. Build before testing.",
        profile_dir().display()
    );

    let patterns = windows_resource_patterns();
    let missing = uncovered(&libraries, &patterns);
    assert!(
        missing.is_empty(),
        "src-tauri/tauri.windows.conf.json does not bundle {missing:?}.\n\
         The installer would leave them behind and the app would not start.\n\
         Libraries present: {libraries:?}\n\
         Patterns configured: {patterns:?}"
    );
}

/// Tauri resolves these patterns relative to `src-tauri/`, and `tauri build`
/// compiles into `target/release`. A pattern aimed anywhere else would either
/// match nothing (a hard bundler error, which at least fails loudly) or match a
/// stale debug library, which would not.
#[test]
fn every_bundle_pattern_reads_from_the_release_build() {
    for pattern in windows_resource_patterns() {
        let dir = pattern.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        assert_eq!(
            dir, "target/release",
            "bundle pattern {pattern:?} does not read from target/release"
        );
    }
}

/// Proof the check above can fail, and that the wildcard matcher is a matcher
/// rather than something that says yes. A passing test proves nothing until it
/// has been seen to fail.
#[test]
fn the_coverage_check_reports_a_library_no_pattern_names() {
    let libraries: BTreeSet<String> = ["onnxruntime.dll", "sherpa-onnx-c-api.dll", "cudart64.dll"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let patterns = vec![
        "target/release/onnxruntime*.dll".to_string(),
        "target/release/sherpa-onnx*.dll".to_string(),
    ];
    assert_eq!(uncovered(&libraries, &patterns), vec!["cudart64.dll"]);
}

#[test]
fn the_wildcard_matcher_anchors_both_ends() {
    assert!(matches("onnxruntime*.dll", "onnxruntime.dll"));
    assert!(matches(
        "onnxruntime*.dll",
        "onnxruntime_providers_shared.dll"
    ));
    assert!(matches("sherpa-onnx*.dll", "sherpa-onnx-cxx-api.dll"));
    assert!(matches("*.dll", "anything.dll"));
    assert!(matches("exact.dll", "exact.dll"));

    assert!(!matches("onnxruntime*.dll", "libonnxruntime.dll"), "prefix");
    assert!(!matches("onnxruntime*.dll", "onnxruntime.so"), "suffix");
    assert!(!matches("sherpa-onnx*.dll", "sherpa.dll"), "middle");
    assert!(!matches("exact.dll", "exact.dll.bak"), "literal");
}

/// The name translation is the part that lets a Linux machine check a Windows
/// installer at all, so it gets its own test.
#[test]
fn shared_library_names_translate_to_their_windows_spelling() {
    assert_eq!(windows_dll_name("libonnxruntime.so"), "onnxruntime.dll");
    assert_eq!(windows_dll_name("libonnxruntime.dylib"), "onnxruntime.dll");
    assert_eq!(windows_dll_name("onnxruntime.dll"), "onnxruntime.dll");
    assert_eq!(
        windows_dll_name("libsherpa-onnx-c-api.so"),
        "sherpa-onnx-c-api.dll"
    );
}
