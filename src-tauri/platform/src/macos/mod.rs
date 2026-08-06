//! macOS: CoreGraphics for idle time, ScreenCaptureKit for system audio.
//!
//! The microphone is not here — `cpal` covers both platforms, so it lives in
//! [`crate::mic`] and needs nothing macOS-specific.
//!
//! # System audio, written 2026-08-05
//!
//! The build order for this plan was Windows plus CI first and macOS after,
//! because Windows is the platform whose capture code can be compile-verified
//! *and* CI-tested without the hardware. For most of that time [`speaker`] held
//! a design and a deliberate error return rather than a guess at an
//! implementation, and a Mac recorded in-person mode while declining meeting
//! mode with a plain-English message.
//!
//! It is implemented now, on a real Mac. An error from it still means what
//! core's `CaptureSources::system` contract says — "I cannot capture the other
//! side of a call", so meeting mode refuses rather than silently recording half
//! a conversation — but the case that now produces one is a refused **Screen
//! Recording** permission, and the message says where to grant it.

pub mod mic_activity;
pub mod power;
pub mod speaker;
