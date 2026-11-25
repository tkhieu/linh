//! Teams/Zoom app-target detection. Not yet implemented on Windows.

use linh_core::Browser;

use super::dirs::Dirs;

// ported in a later change: Windows app-target discovery (registered
// URL protocol handlers) is not implemented yet.
pub fn find_app(_dirs: &Dirs, _app_id: &str) -> Option<Browser> {
    None
}

pub fn is_app_installed(dirs: &Dirs, app_id: &str) -> bool {
    find_app(dirs, app_id).is_some()
}
