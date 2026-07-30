//! Notetaker's platform layer: the microphone, system audio, and the machine's
//! idle and power state, per operating system.
//!
//! # Why this is a separate crate
//!
//! `notetaker-core` owns the `AudioSource` and `SystemProbe` traits and
//! implements them for the concrete types exported here. The dependency runs
//! **core -> platform**, never the reverse, and this crate depends on no other
//! notetaker crate at all.
//!
//! That is not architectural taste, it is the verification story. Core pulls in
//! bundled SQLite, whisper.cpp and sherpa-onnx, all of which need a C++
//! toolchain for the target platform. This crate's dependencies are pure Rust,
//! and `cargo check` does not link — so the platform code type-checks against
//! real foreign targets from an ordinary Linux box with no cross-compiler and
//! no SDK:
//!
//! ```text
//! cargo check -p notetaker-platform --target x86_64-pc-windows-msvc
//! cargo check -p notetaker-platform --target aarch64-apple-darwin
//! ```
//!
//! Without that, every line of platform code would be written blind and first
//! compiled on hardware. With it, the dominant failure mode — it does not
//! build — is caught in seconds.
//!
//! # What is and is not verified
//!
//! The honest split, because it matters when something misbehaves on real
//! hardware:
//!
//! - [`convert`], [`resample`] and [`ring`] are pure, platform-independent, and
//!   **fully tested on Linux**. This is deliberate and it is where the bugs
//!   would otherwise be: a wrong stride, the wrong endianness, aliasing from a
//!   careless downsample. All of those produce plausible-sounding *audio*
//!   rather than an error, so none of them can be left to be discovered by ear.
//! - The per-OS modules are **compile-verified, run-unverified** until they
//!   reach real hardware or CI. What they cannot prove is that the OS returns
//!   the data its documentation promises.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.). See the NOTICE file for the terms and the per-file provenance.

pub mod convert;
pub mod resample;
pub mod ring;

/// The microphone. `cpal` covers both shipping platforms, so there is no per-OS
/// split here — and it is absent on Linux, where `cpal` is not built at all.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod mic;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use mic::MicSource;

/// The sample rate everything here converts to.
///
/// Must equal `notetaker_core::capture::SAMPLE_RATE`. It is repeated rather
/// than imported because this crate deliberately has no dependency on core —
/// and pinned against it by a test in core, so the two cannot drift apart
/// silently.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos::power::read_power_state;
/// The platform's power/idle probe, on platforms that have one.
#[cfg(target_os = "windows")]
pub use windows::power::read_power_state;

/// A reading of the machine's idle time and power state.
///
/// Mirrors `notetaker_core::power::PowerState`; core converts between them.
/// Duplicated for the same reason as [`TARGET_SAMPLE_RATE`] — no dependency on
/// core — and likewise pinned by a test there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPowerState {
    /// Seconds since the last keyboard or mouse event.
    pub idle_secs: u64,
    /// True on wall power rather than battery.
    pub on_ac: bool,
    /// Battery percentage, or `None` on a machine with no battery.
    pub battery_pct: Option<u8>,
}
