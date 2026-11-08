//! OS-specific mechanics, isolated behind `#[cfg(target_os = "...")]`
//! modules so the rest of `src-tauri` stays platform-agnostic.

#[cfg(target_os = "linux")]
pub mod linux;
