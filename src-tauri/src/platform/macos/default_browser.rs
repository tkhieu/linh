//! Default-browser get/set. Not yet implemented on macOS.

use linh_core::Browser;

pub fn current_default_browser_id() -> Option<String> {
    None
}

pub fn is_default_browser() -> bool {
    false
}

// ported in a later change: setting the default browser on macOS needs
// LaunchServices (`LSSetDefaultHandlerForURLScheme`) plus an
// Info.plist-declared URL type.
pub fn set_default_browser() -> Result<(), String> {
    Err("setting the default browser is not yet supported on macOS".to_string())
}

pub fn matches_desktop_id(_browser: &Browser, _id: &str) -> bool {
    false
}
