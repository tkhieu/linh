//! Installed-browser detection. Not yet implemented on macOS.

use linh_core::Browser;

use super::dirs::Dirs;

// ported in a later change: macOS browser discovery (LaunchServices
// handlers for http/https) is not implemented yet.
pub fn detect_browsers(_dirs: &Dirs) -> Vec<Browser> {
    Vec::new()
}
