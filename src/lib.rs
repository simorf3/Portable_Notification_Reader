//! Portable Notification Reader – library crate.
//!
//! Cross-platform modules (config, networking, parsing, filtering) build on any
//! OS so the logic can be unit-tested off-Windows. The UI / audio / SAPI modules
//! are Windows-only.

pub mod config;
pub mod drm;
pub mod edge_tts;
pub mod filter;
pub mod locale;
pub mod notifications;
pub mod voices;

#[cfg(windows)]
pub mod speech;
#[cfg(windows)]
pub mod worker;
#[cfg(windows)]
pub mod app;
