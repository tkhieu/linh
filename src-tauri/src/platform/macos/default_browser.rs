//! Default-browser get/set via LaunchServices.

use linh_core::Browser;

use crate::updates::{current_dist, Dist};

use super::dirs::Dirs;
use super::launch_services;
use super::{workspace, LINH_BUNDLE_ID};

/// Whichever bundle id is currently registered as `http`'s default
/// handler (used by `app_config` to carry a previous default forward
/// into a freshly created config).
pub fn current_default_browser_id() -> Option<String> {
    launch_services::default_handler_for_scheme("http")
}

pub fn is_default_browser() -> bool {
    ["http", "https"].iter().all(|scheme| {
        launch_services::default_handler_for_scheme(scheme)
            .is_some_and(|id| id.eq_ignore_ascii_case(LINH_BUNDLE_ID))
    })
}

/// Requests that Linh become the default `http`/`https` handler. On
/// macOS 12+ this makes the OS show its own confirmation UI rather than
/// switching immediately — see
/// [`launch_services::set_default_handler_for_scheme`]'s doc comment —
/// so `Ok(())` means the request was accepted, not that the switch has
/// actually happened; callers must re-check [`is_default_browser`]
/// rather than assume success from this.
///
/// The Mac App Store build (`LINH_DIST=mas`) takes a different path
/// entirely: `LSSetDefaultHandlerForURLScheme` is a LaunchServices
/// function that *writes* state, which — unlike the read-only `LSCopy*`
/// queries the rest of this module still uses — the sandbox does not
/// let through. `workspace::set_default_application_for_scheme` (built
/// on `NSWorkspace.setDefaultApplication(at:toOpenURLsWithScheme:completion:)`,
/// macOS 12+) is used instead, resolving Linh's own bundle path via the
/// same read-only `app_urls_for_bundle_id` query `browsers.rs` already
/// relies on. Also fire-and-forget for the same reason: its completion
/// handler only notices an outright rejection (and then falls back to
/// opening System Settings' picker directly) rather than confirming
/// success, which must still be checked via [`is_default_browser`].
pub fn set_default_browser() -> Result<(), String> {
    if current_dist() == Dist::Mas {
        let dirs = Dirs::from_env();
        let own_bundle = launch_services::app_urls_for_bundle_id(LINH_BUNDLE_ID)
            .into_iter()
            .find(|path| path.is_dir())
            .ok_or_else(|| "could not resolve Linh's own bundle path".to_string())?;
        let own_bundle = own_bundle.to_string_lossy().into_owned();
        for scheme in ["http", "https"] {
            workspace::set_default_application_for_scheme(&dirs, &own_bundle, scheme);
        }
        return Ok(());
    }

    for scheme in ["http", "https"] {
        launch_services::set_default_handler_for_scheme(scheme, LINH_BUNDLE_ID)?;
    }
    Ok(())
}

/// Bundle ids are compared case-insensitively but otherwise exactly —
/// unlike Windows' ProgID scheme, a macOS bundle id *is* the browser's
/// own `browser.id`, so no family-specific guessing is needed.
pub fn matches_desktop_id(browser: &Browser, id: &str) -> bool {
    browser.id.eq_ignore_ascii_case(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{BrowserFamily, InstallKind};

    fn browser(id: &str) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: "/Applications/Example.app".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: None,
        }
    }

    #[test]
    fn matches_by_bundle_id_case_insensitively() {
        assert!(matches_desktop_id(
            &browser("com.google.chrome"),
            "com.Google.Chrome"
        ));
    }

    #[test]
    fn does_not_match_unrelated_bundle_id() {
        assert!(!matches_desktop_id(
            &browser("org.mozilla.firefox"),
            "com.google.chrome"
        ));
    }
}
