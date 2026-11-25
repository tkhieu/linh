//! OS-specific mechanics, isolated behind `#[cfg(target_os = "...")]`
//! modules so the rest of `src-tauri` stays platform-agnostic.
//!
//! Each OS module (`linux`, `macos`, `windows`) exposes the same set of
//! submodules — `app_config`, `apps`, `browsers`, `default_browser`,
//! `dirs`, `icons`, `notify` — so callers reach the active one through
//! [`current`] rather than naming an OS directly. Only `macos` and
//! `windows` exist as compiling skeletons today; every function in them
//! returns an explicit "unsupported" result instead of a guess.
//!
//! Logic that behaves identically on every OS (process spawning, URL
//! scheme filtering, log-line writing) lives directly here instead of
//! being duplicated per OS.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub use linux as current;
#[cfg(target_os = "macos")]
pub use macos as current;
#[cfg(target_os = "windows")]
pub use windows as current;

pub mod launcher;
pub mod logging;
pub mod url_intake;
