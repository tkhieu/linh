//! Chromium profile discovery from each browser's `Local State` file
//! under `~/Library/Application Support/<vendor>/<product>`.

use std::fs;
use std::path::PathBuf;

use linh_core::BrowserProfile;

/// Maps a known Chromium-family bundle id (already lowercased) to its
/// `~/Library/Application Support/<subdir>`.
fn vendor_subdir(bundle_id: &str) -> Option<&'static str> {
    match bundle_id {
        "com.google.chrome" => Some("Google/Chrome"),
        "com.google.chrome.canary" => Some("Google/Chrome Canary"),
        "com.google.chrome.beta" => Some("Google/Chrome Beta"),
        "com.microsoft.edgemac" => Some("Microsoft Edge"),
        "com.microsoft.edgemac.beta" => Some("Microsoft Edge Beta"),
        "com.brave.browser" => Some("BraveSoftware/Brave-Browser"),
        "com.brave.browser.beta" => Some("BraveSoftware/Brave-Browser-Beta"),
        "com.vivaldi.vivaldi" => Some("Vivaldi"),
        "com.operasoftware.opera" => Some("com.operasoftware.Opera"),
        "company.thebrowser.browser" => Some("Arc"),
        _ => None,
    }
}

/// Returns the Chromium profiles for `bundle_id` (already lowercased),
/// empty if it's not a known Chromium-family bundle id or its `Local
/// State` file is missing/malformed.
pub fn profiles_for(bundle_id: &str) -> Vec<BrowserProfile> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let Some(subdir) = vendor_subdir(bundle_id) else {
        return Vec::new();
    };
    read_profiles(
        &home
            .join("Library/Application Support")
            .join(subdir)
            .join("Local State"),
    )
}

fn read_profiles(path: &std::path::Path) -> Vec<BrowserProfile> {
    let Ok(data) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) else {
        return Vec::new();
    };
    let Some(info_cache) = json
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };

    let mut profiles: Vec<BrowserProfile> = info_cache
        .iter()
        .map(|(dir, info)| BrowserProfile {
            dir: dir.clone(),
            display_name: info
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(dir)
                .to_string(),
        })
        .collect();
    profiles.sort_by(|a, b| a.dir.cmp(&b.dir));
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_bundle_ids_map_to_their_vendor_subdir() {
        assert_eq!(vendor_subdir("com.google.chrome"), Some("Google/Chrome"));
        assert_eq!(
            vendor_subdir("com.brave.browser"),
            Some("BraveSoftware/Brave-Browser")
        );
        assert_eq!(vendor_subdir("org.mozilla.firefox"), None);
        assert_eq!(vendor_subdir("com.apple.safari"), None);
    }

    #[test]
    fn reads_profiles_from_local_state() {
        let root =
            std::env::temp_dir().join(format!("linh-macos-chromium-test-{}", std::process::id()));
        let dir = root.join("Library/Application Support/Google/Chrome");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"Person 1"},"Profile 1":{"name":"Work"}}}}"#,
        )
        .unwrap();

        let mut profiles = read_profiles(&dir.join("Local State"));
        profiles.sort_by(|a, b| a.dir.cmp(&b.dir));
        assert_eq!(
            profiles,
            vec![
                BrowserProfile {
                    dir: "Default".to_string(),
                    display_name: "Person 1".to_string()
                },
                BrowserProfile {
                    dir: "Profile 1".to_string(),
                    display_name: "Work".to_string()
                },
            ]
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_local_state_yields_no_profiles() {
        assert!(read_profiles(std::path::Path::new("/nonexistent/Local State")).is_empty());
    }

    #[test]
    fn malformed_local_state_yields_no_profiles() {
        let root = std::env::temp_dir().join(format!(
            "linh-macos-chromium-malformed-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("Local State");
        fs::write(&path, "not json").unwrap();
        assert!(read_profiles(&path).is_empty());
        fs::remove_dir_all(&root).ok();
    }
}
