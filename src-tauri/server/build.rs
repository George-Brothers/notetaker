fn main() {
    // `notetaker-serve` has to find sherpa's dylibs beside itself.
    //
    // `sherpa-rs` copies ONNX Runtime and its own shared libraries loose into
    // `target/<profile>/` and links them by install name, so the binary asks
    // the loader for `@rpath/libonnxruntime.1.17.1.dylib`. Nothing adds an
    // `LC_RPATH` to an ordinary cargo binary, and macOS's loader — unlike
    // Linux's, which at least has a system search path to fall back on —
    // fails outright with "no LC_RPATH's found".
    //
    // The app crate's `build.rs` does the same thing, but `rustc-link-arg-bins`
    // only reaches the binaries of the package that emits it. This is a
    // separate package, so it needs its own. Without it `notetaker-serve`
    // cannot be started on a Mac without `DYLD_LIBRARY_PATH` set by hand,
    // which is not a thing a shipped binary should require.
    //
    // `CARGO_CFG_TARGET_OS` rather than `cfg!(target_os = "macos")`: a build
    // script is compiled for the *host*, so `cfg!` would answer "what am I
    // running on" instead of "what am I building for".
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path");
    }
}
