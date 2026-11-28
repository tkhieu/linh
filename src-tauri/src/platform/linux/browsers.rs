use linh_core::BrowserFamily;

use super::chromium;
use super::dirs::Dirs;
use super::scan;

/// Detects installed http(s)-capable browsers, including their Chromium
/// profiles where applicable.
pub fn detect_browsers(dirs: &Dirs) -> Vec<linh_core::Browser> {
    let mut browsers = scan::scan_browsers(dirs);
    populate_profiles(&mut browsers, dirs);
    browsers
}

pub fn detect_browsers_and_aliases(dirs: &Dirs) -> scan::BrowserDiscovery {
    let mut discovery = scan::scan_browser_discovery(dirs);
    populate_profiles(&mut discovery.browsers, dirs);
    discovery
}

fn populate_profiles(browsers: &mut [linh_core::Browser], dirs: &Dirs) {
    for browser in browsers {
        if browser.family == BrowserFamily::Chromium {
            browser.profiles = chromium::profiles_for(browser, dirs);
        }
    }
}
