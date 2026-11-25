//! Default-browser get/set. Not yet implemented on Windows.

use linh_core::Browser;

pub fn current_default_browser_id() -> Option<String> {
    None
}

pub fn is_default_browser() -> bool {
    false
}

// ported in a later change: setting the default browser on Windows needs
// the registry `UrlAssociations`/`StartMenuInternet` dance and (on
// Windows 10+) sending the user through the Settings > Default Apps UI.
pub fn set_default_browser() -> Result<(), String> {
    Err("setting the default browser is not yet supported on Windows".to_string())
}

pub fn matches_desktop_id(_browser: &Browser, _id: &str) -> bool {
    false
}
