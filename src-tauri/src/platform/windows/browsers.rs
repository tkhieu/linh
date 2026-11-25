//! Installed-browser detection. Not yet implemented on Windows.

use linh_core::Browser;

use super::dirs::Dirs;

// ported in a later change: Windows browser discovery
// (`HKLM/HKCU\...\StartMenuInternet`) is not implemented yet.
pub fn detect_browsers(_dirs: &Dirs) -> Vec<Browser> {
    Vec::new()
}
