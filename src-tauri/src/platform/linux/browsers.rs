use linh_core::BrowserFamily;

use super::chromium;
use super::dirs::Dirs;
use super::scan;

/// Detects installed http(s)-capable browsers, including their Chromium
/// profiles where applicable.
pub fn detect_browsers(dirs: &Dirs) -> Vec<linh_core::Browser> {
    let mut browsers = scan::scan_browsers(dirs);
    for browser in &mut browsers {
        if browser.family == BrowserFamily::Chromium {
            browser.profiles = chromium::profiles_for(browser, dirs);
        }
    }
    browsers
}
