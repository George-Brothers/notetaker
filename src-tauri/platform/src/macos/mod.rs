//! macOS: CoreGraphics for idle time, ScreenCaptureKit for system audio.
//!
//! The microphone is not here — `cpal` covers both platforms, so it lives in
//! [`crate::mic`] and needs nothing macOS-specific.
//!
//! # System audio is deliberately not implemented yet
//!
//! Windows capture is implemented first because it can be compile-verified and
//! CI-tested without macOS hardware. [`speaker`] therefore holds the design and
//! the reason it is not written blind, not a guess at an implementation.
//!
//! In the meantime a Mac records in-person mode normally and declines meeting
//! mode with a plain-English message. That is core's existing
//! `CaptureSources::system` contract — an error there means "I cannot capture
//! the other side of a call", and meeting mode refuses rather than silently
//! recording half a conversation. Deliberately not changed here.

pub mod power;
pub mod speaker;
