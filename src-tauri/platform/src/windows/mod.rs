//! Windows: WASAPI loopback for system audio, Win32 for idle and power state.
//!
//! The microphone is not here — `cpal` covers both platforms, so it lives in
//! [`crate::mic`].

pub mod mic_activity;
pub mod power;
pub mod speaker;

pub use speaker::SystemAudioSource;
