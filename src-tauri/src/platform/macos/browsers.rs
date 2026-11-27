//! Installed-browser detection via LaunchServices' registered `https`
//! handlers.

use std::collections::BTreeMap;
use std::path::Path;

use linh_core::{Browser, BrowserFamily, InstallKind};

use super::chromium;
use super::dirs::Dirs;
use super::launch_services;
use super::{LINH_BUNDLE_ID, LINH_BUNDLE_ID_LEGACY};

/// True for either of Linh's own bundle ids (current or pre-rename
/// legacy), so a stale LaunchServices registration left over from before
/// the `dev.tkhieu.linh` -> `dev.hieutk.linh` rename is never listed as a
/// separate, installable browser.
fn is_self(bundle_id: &str) -> bool {
    bundle_id.eq_ignore_ascii_case(LINH_BUNDLE_ID)
        || bundle_id.eq_ignore_ascii_case(LINH_BUNDLE_ID_LEGACY)
}

/// Classifies a (lowercased) bundle id into its engine family and
/// whether it has a private/incognito launch flag. Safari has neither a
/// known family nor a CLI private-mode flag, so it (and anything else
/// not in this list) falls into `Other`/`false` — still shown as a
/// browser, just without private-window support, same as an unrecognized
/// browser on Linux/Windows.
fn classify(bundle_id: &str) -> (BrowserFamily, bool) {
    match bundle_id {
        "com.google.chrome" | "com.google.chrome.beta" | "com.google.chrome.canary" => {
            (BrowserFamily::Chromium, true)
        }
        "com.microsoft.edgemac" | "com.microsoft.edgemac.beta" => (BrowserFamily::Chromium, true),
        "com.brave.browser" | "com.brave.browser.beta" => (BrowserFamily::Chromium, true),
        "com.vivaldi.vivaldi" => (BrowserFamily::Chromium, true),
        "com.operasoftware.opera" => (BrowserFamily::Chromium, true),
        // Arc.
        "company.thebrowser.browser" => (BrowserFamily::Chromium, true),
        "org.mozilla.firefox" => (BrowserFamily::Firefox, true),
        _ => (BrowserFamily::Other, false),
    }
}

/// `info_name` (from `Info.plist`) if present, else the bundle's file
/// name without its `.app` extension, else the bundle id itself.
fn display_name_for(bundle_id: &str, info_name: Option<String>, bundle_path: &Path) -> String {
    info_name
        .or_else(|| {
            bundle_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| bundle_id.to_string())
}

pub fn detect_browsers(_dirs: &Dirs) -> Vec<Browser> {
    let mut results: BTreeMap<String, Browser> = BTreeMap::new();

    for bundle_id in launch_services::handlers_for_scheme("https") {
        let bundle_id = bundle_id.to_ascii_lowercase();
        if is_self(&bundle_id) || results.contains_key(&bundle_id) {
            continue;
        }
        let Some(bundle_path) = launch_services::app_urls_for_bundle_id(&bundle_id)
            .into_iter()
            .find(|path| path.is_dir())
        else {
            continue;
        };
        let info = super::bundle::read_info_plist(&bundle_path);
        let (family, supports_private) = classify(&bundle_id);
        let display_name = display_name_for(
            &bundle_id,
            info.as_ref().and_then(|i| i.display_name.clone()),
            &bundle_path,
        );
        let bundle_path_str = bundle_path.to_string_lossy().into_owned();

        let mut browser = Browser {
            id: bundle_id.clone(),
            display_name,
            family,
            install: InstallKind::Native {
                exec: bundle_path_str.clone(),
            },
            profiles: Vec::new(),
            supports_private,
            icon: Some(bundle_path_str),
            desktop_file: None,
        };
        if family == BrowserFamily::Chromium {
            browser.profiles = chromium::profiles_for(&bundle_id);
        }
        results.insert(bundle_id, browser);
    }

    results.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_self_matches_current_and_legacy_bundle_id() {
        assert!(is_self(LINH_BUNDLE_ID));
        assert!(is_self(LINH_BUNDLE_ID_LEGACY));
        assert!(is_self("DEV.HIEUTK.LINH"));
        assert!(!is_self("com.google.chrome"));
    }

    #[test]
    fn known_browser_families() {
        for id in [
            "com.google.chrome",
            "com.microsoft.edgemac",
            "com.brave.browser",
            "com.vivaldi.vivaldi",
            "com.operasoftware.opera",
            "company.thebrowser.browser",
        ] {
            assert_eq!(classify(id), (BrowserFamily::Chromium, true));
        }
        assert_eq!(
            classify("org.mozilla.firefox"),
            (BrowserFamily::Firefox, true)
        );
    }

    #[test]
    fn safari_has_no_private_flag() {
        assert_eq!(classify("com.apple.safari"), (BrowserFamily::Other, false));
    }

    #[test]
    fn unknown_bundle_id_is_other_without_private() {
        assert_eq!(
            classify("com.example.somebrowser"),
            (BrowserFamily::Other, false)
        );
    }

    #[test]
    fn display_name_prefers_info_plist_name() {
        assert_eq!(
            display_name_for(
                "com.google.chrome",
                Some("Google Chrome".to_string()),
                Path::new("/Applications/Google Chrome.app")
            ),
            "Google Chrome"
        );
    }

    #[test]
    fn display_name_falls_back_to_bundle_file_stem() {
        assert_eq!(
            display_name_for(
                "com.example.app",
                None,
                Path::new("/Applications/Some App.app")
            ),
            "Some App"
        );
    }

    #[test]
    fn display_name_falls_back_to_bundle_id_as_last_resort() {
        assert_eq!(
            display_name_for("com.example.app", None, Path::new("")),
            "com.example.app"
        );
    }
}
