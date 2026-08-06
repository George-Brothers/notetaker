fn main() {
    // Where the app looks for its dylibs.
    //
    // `sherpa-rs` copies ONNX Runtime and sherpa's own shared libraries loose
    // into `target/<profile>/`, and every one of them is linked by *install
    // name* — the binary asks the loader for `@rpath/libonnxruntime.1.17.1.dylib`
    // rather than for a path. Nothing otherwise adds an `LC_RPATH` to a plain
    // cargo binary, which is why `notetaker-serve` only runs with
    // `DYLD_LIBRARY_PATH` set by hand.
    //
    // Only `@executable_path` is added here, deliberately. `tauri-build`
    // already emits `@executable_path/../Frameworks` — the one the bundled
    // `.app` needs, since its executable sits in `Contents/MacOS/` while the
    // libraries land in `Contents/Frameworks/`. Adding it a second time makes
    // the linker warn about a duplicate rpath, and CI denies warnings. This was
    // read off the built binary rather than assumed: `otool -l` on
    // `Notetaker.app` shows both entries with this line absent.
    //
    // What Tauri does not cover is running the binary straight out of
    // `target/<profile>/`, where sherpa's dylibs sit beside it — `cargo run`,
    // `tauri dev`, and `notetaker-serve`. That is what this adds.
    //
    // `CARGO_CFG_TARGET_OS` rather than `cfg!(target_os = "macos")`: a build
    // script is compiled for the *host*, so `cfg!` here would answer "what am I
    // running on", not "what am I building for". They differ the moment anyone
    // cross-compiles, and the failure would be silent.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path");
    }

    tauri_build::build()
}
