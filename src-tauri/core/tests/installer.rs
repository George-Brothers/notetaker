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
//!
//! **And that is the limit of what Linux can tell us.** The first run of this
//! test on Windows found a fifth library Linux does not have: `cargs.dll`,
//! which whisper.cpp's CMake builds as a shared library there and links
//! statically here. So this test is a fast first filter, not the last word —
//! the Windows job runs the same test against the real file set, and the
//! packaging job then asks the built binaries what they actually import.

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

// --- macOS ---------------------------------------------------------------
//
// The same failure, one bundle format over. A `.app` puts its executable in
// `Contents/MacOS/` and its libraries in `Contents/Frameworks/`, so the Windows
// fix does not carry over: `bundle.resources` is the wrong key and would drop
// the dylibs into `Contents/Resources/`, where the loader never looks.
//
// Two things make this **more** dangerous than the Windows side, not less:
//
// 1. `bundle.macOS.frameworks` takes **literal paths, not globs.** The property
//    the Windows config leans on — "a Tauri glob that matches nothing is a hard
//    bundler error, so an empty installer cannot be built" — does not exist
//    here. A wrong or stale entry is simply a file that is not copied. This
//    test is the only thing standing in that gap.
// 2. The list must name `libonnxruntime.1.17.1.dylib`, the **versioned** file.
//    `libonnxruntime.dylib` beside it is a symlink, and every binary that wants
//    ONNX Runtime asks the loader for the versioned install name. Bundling only
//    the unversioned symlink produces an app that launches and then dies the
//    first time it transcribes anything. A version bump renames that file and
//    this test is what catches it.

/// macOS's file name for a shared library, whatever platform built it.
///
/// The mirror of [`windows_dll_name`], and the reason both exist: this test
/// runs on all three platforms, and only the one matching the host sees the
/// real file set. `libonnxruntime.so` and `onnxruntime.dll` are both
/// `libonnxruntime.dylib`; on a Mac the mapping is the identity.
///
/// Version numbers are deliberately **not** invented — `onnxruntime.dll` maps
/// to `libonnxruntime.dylib`, not to `libonnxruntime.1.17.1.dylib`, because
/// nothing off-Mac knows the version. That is exactly why the coverage check
/// below matches on the unversioned stem rather than on the whole name.
fn macos_dylib_name(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    // `libonnxruntime.1.17.1.dylib` has file stem `libonnxruntime.1.17.1`.
    let stem = stem.split('.').next().unwrap_or(stem);
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    format!("lib{stem}.dylib")
}

/// Every third-party shared library beside our binaries, in its macOS spelling.
fn shared_libraries_in_macos_spelling() -> BTreeSet<String> {
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
        let dylib = macos_dylib_name(name);
        if dylib.starts_with("libnotetaker") {
            continue;
        }
        found.insert(dylib);
    }
    found
}

/// The `bundle.macOS.frameworks` list of the macOS-only Tauri config.
fn macos_framework_entries() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tauri.macos.conf.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading the macOS bundle config at {}: {e}", path.display()));
    let config: serde_json::Value =
        serde_json::from_str(&text).expect("tauri.macos.conf.json is not valid JSON");

    config["bundle"]["macOS"]["frameworks"]
        .as_array()
        .expect("bundle.macOS.frameworks must be an array of paths")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("every frameworks entry is a path string")
                .to_string()
        })
        .collect()
}

#[test]
fn the_mac_app_bundles_every_shared_library_it_needs() {
    let libraries = shared_libraries_in_macos_spelling();
    assert!(
        !libraries.is_empty(),
        "no shared libraries were found in {} — this test would pass vacuously, \
         which is worse than failing. Build before testing.",
        profile_dir().display()
    );

    let bundled: BTreeSet<String> = macos_framework_entries()
        .iter()
        .filter_map(|p| p.rsplit('/').next().map(macos_dylib_name))
        .collect();

    let missing: Vec<&String> = libraries.difference(&bundled).collect();
    assert!(
        missing.is_empty(),
        "src-tauri/tauri.macos.conf.json does not bundle {missing:?}.\n\
         `bundle.macOS.frameworks` takes literal paths and does not glob, so a \
         library that is not named here is simply left out of Contents/Frameworks \
         and the app dies on launch.\n\
         Libraries present: {libraries:?}\n\
         Bundled (unversioned): {bundled:?}"
    );
}

/// Tauri resolves these relative to `src-tauri/`, and `tauri build` compiles
/// into `target/release`. The Windows config has the same rule and the same
/// test; unlike Windows, a path pointing nowhere here fails quietly.
#[test]
fn every_macos_framework_entry_reads_from_the_release_build() {
    for entry in macos_framework_entries() {
        let dir = entry.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        assert_eq!(
            dir, "target/release",
            "framework entry {entry:?} does not read from target/release"
        );
    }
}

/// The trap that makes this whole file worth having on macOS.
///
/// `libonnxruntime.dylib` is a symlink; the versioned file beside it is what
/// every binary's `LC_LOAD_DYLIB` actually names. Bundling the symlink instead
/// of the real file is the one mistake that still produces a `.app` which
/// builds, installs, launches, and then fails the first time it transcribes.
#[test]
fn the_bundled_onnxruntime_is_the_versioned_file_not_the_symlink() {
    let entries = macos_framework_entries();
    let onnx: Vec<&String> = entries
        .iter()
        .filter(|e| e.contains("libonnxruntime"))
        .collect();

    assert_eq!(
        onnx.len(),
        1,
        "expected exactly one ONNX Runtime entry, got {onnx:?}"
    );
    let name = onnx[0].rsplit('/').next().unwrap_or(onnx[0]);
    assert_ne!(
        name, "libonnxruntime.dylib",
        "bundling the unversioned symlink copies a dangling link into \
         Contents/Frameworks. Binaries load @rpath/libonnxruntime.<version>.dylib, \
         so the real versioned file is the one that has to be there."
    );
    assert!(
        name.starts_with("libonnxruntime.") && name.ends_with(".dylib"),
        "{name:?} does not look like a versioned ONNX Runtime dylib"
    );
}

/// Proof the macOS coverage check can fail, matching the Windows one. A passing
/// test proves nothing until it has been seen to fail.
#[test]
fn the_macos_coverage_check_reports_a_library_no_entry_names() {
    let libraries: BTreeSet<String> = [
        "libonnxruntime.dylib",
        "libsherpa-onnx-c-api.dylib",
        "libcudart.dylib",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let bundled: BTreeSet<String> = ["libonnxruntime.dylib", "libsherpa-onnx-c-api.dylib"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let missing: Vec<&String> = libraries.difference(&bundled).collect();
    assert_eq!(missing, vec!["libcudart.dylib"]);
}

/// The version-stripping is the subtle half of the translation, so it gets its
/// own test: a Linux or Windows build must map onto the *same* key the macOS
/// config produces, or the coverage check compares two disjoint sets and passes
/// while proving nothing.
#[test]
fn shared_library_names_translate_to_their_macos_spelling() {
    assert_eq!(macos_dylib_name("libonnxruntime.so"), "libonnxruntime.dylib");
    assert_eq!(macos_dylib_name("onnxruntime.dll"), "libonnxruntime.dylib");
    assert_eq!(
        macos_dylib_name("libonnxruntime.1.17.1.dylib"),
        "libonnxruntime.dylib"
    );
    assert_eq!(
        macos_dylib_name("libonnxruntime.dylib"),
        "libonnxruntime.dylib"
    );
    assert_eq!(
        macos_dylib_name("libsherpa-onnx-c-api.so"),
        "libsherpa-onnx-c-api.dylib"
    );
    assert_eq!(
        macos_dylib_name("sherpa-onnx-c-api.dll"),
        "libsherpa-onnx-c-api.dylib"
    );
}
