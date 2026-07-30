//! macOS: CoreGraphics for idle time, ScreenCaptureKit for system audio.
//!
//! The microphone is not here — `cpal` covers both platforms, so it lives in
//! [`crate::mic`] and needs nothing macOS-specific.
//!
//! # System audio is deliberately not implemented yet
//!
//! Mr. Brothers' build order for this plan was Windows plus CI first, macOS
//! after, because the Mac hardware only arrives ~2026-07-30 and Windows is the
//! platform whose capture code can be compile-verified *and* CI-tested without
//! it. [`speaker`] therefore holds the design and the reason it is not written
//! blind, not a guess at an implementation.
//!
//! Nothing breaks in the meantime: core already has `SilentSource` for exactly
//! this case, so meeting mode on a Mac records a real microphone track and an
//! empty system track until [`speaker`] lands. That is a documented, existing
//! fallback rather than a new one invented here.

pub mod power;
pub mod speaker;
