use std::fs;
use std::path::{Path, PathBuf};

use linh_core::{Browser, BrowserProfile, InstallKind};

use super::dirs::Dirs;

/// Returns the Chromium profiles for `browser` (empty if its `Local
/// State` file is missing or malformed, or if its binary/app id is not a
/// recognized Chromium build).
pub fn profiles_for(browser: &Browser, dirs: &Dirs) -> Vec<BrowserProfile> {
    let Some(path) = local_state_path(browser, dirs) else {
        return Vec::new();
    };
    read_profiles(&path)
}

fn local_state_path(browser: &Browser, dirs: &Dirs) -> Option<PathBuf> {
    match &browser.install {
        InstallKind::Native { exec } => {
            let subdir = native_config_subdir(&browser.id, exec)?;
            Some(dirs.home.join(".config").join(subdir).join("Local State"))
        }
        InstallKind::Flatpak { app_id } => {
            let subdir = flatpak_config_subdir(app_id)?;
            Some(
                dirs.home
                    .join(".var/app")
                    .join(app_id)
                    .join("config")
                    .join(subdir)
                    .join("Local State"),
            )
        }
    }
}

/// Maps a native Chromium-family browser to its `~/.config/<subdir>`.
fn native_config_subdir(id: &str, exec: &str) -> Option<&'static str> {
    let haystack = format!("{} {}", id.to_lowercase(), exec.to_lowercase());
    if haystack.contains("chrome-beta") {
        Some("google-chrome-beta")
    } else if haystack.contains("chrome") {
        Some("google-chrome")
    } else if haystack.contains("chromium") {
        Some("chromium")
    } else if haystack.contains("brave") {
        Some("BraveSoftware/Brave-Browser")
    } else if haystack.contains("edge") {
        Some("microsoft-edge")
    } else if haystack.contains("vivaldi") {
        Some("vivaldi")
    } else {
        None
    }
}

/// Maps a known Chromium-family Flatpak app id to the same `<subdir>`
/// convention, scoped under `~/.var/app/<app_id>/config/`.
fn flatpak_config_subdir(app_id: &str) -> Option<&'static str> {
    match app_id {
        "com.google.Chrome" => Some("google-chrome"),
        "com.brave.Browser" => Some("BraveSoftware/Brave-Browser"),
        "com.microsoft.Edge" => Some("microsoft-edge"),
        "com.vivaldi.Vivaldi" => Some("vivaldi"),
        "org.chromium.Chromium" => Some("chromium"),
        _ => None,
    }
}

fn read_profiles(path: &Path) -> Vec<BrowserProfile> {
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
        .map(|(dir, info)| {
            let display_name = info
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(dir)
                .to_string();
            BrowserProfile {
                dir: dir.clone(),
                display_name,
            }
        })
        .collect();
    profiles.sort_by(|a, b| a.dir.cmp(&b.dir));
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::BrowserFamily;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-chromium-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn native_browser(id: &str, exec: &str) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: exec.to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: None,
        }
    }

    #[test]
    fn reads_profiles_from_native_local_state() {
        let root = temp_root("native");
        let local_state_dir = root.join(".config/google-chrome");
        fs::create_dir_all(&local_state_dir).unwrap();
        fs::write(
            local_state_dir.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"Person 1"},"Profile 1":{"name":"Work"}}}}"#,
        )
        .unwrap();

        let dirs = Dirs {
            home: root.clone(),
            config_home: root.join(".config"),
            state_home: root.join(".local/state"),
            application_dirs: Vec::new(),
        };
        let browser = native_browser("google-chrome", "/usr/bin/google-chrome-stable %U");
        let mut profiles = profiles_for(&browser, &dirs);
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
    fn reads_profiles_from_flatpak_local_state() {
        let root = temp_root("flatpak");
        let local_state_dir = root.join(".var/app/com.google.Chrome/config/google-chrome");
        fs::create_dir_all(&local_state_dir).unwrap();
        fs::write(
            local_state_dir.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"Person 1"}}}}"#,
        )
        .unwrap();

        let dirs = Dirs {
            home: root.clone(),
            config_home: root.join(".config"),
            state_home: root.join(".local/state"),
            application_dirs: Vec::new(),
        };
        let browser = Browser {
            id: "com.google.Chrome".to_string(),
            display_name: "Chrome".to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Flatpak {
                app_id: "com.google.Chrome".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: None,
        };
        let profiles = profiles_for(&browser, &dirs);
        assert_eq!(
            profiles,
            vec![BrowserProfile {
                dir: "Default".to_string(),
                display_name: "Person 1".to_string()
            }]
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_local_state_yields_no_profiles() {
        let root = temp_root("missing");
        let dirs = Dirs {
            home: root.clone(),
            config_home: root.join(".config"),
            state_home: root.join(".local/state"),
            application_dirs: Vec::new(),
        };
        let browser = native_browser("google-chrome", "/usr/bin/google-chrome-stable %U");
        assert!(profiles_for(&browser, &dirs).is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_local_state_yields_no_profiles() {
        let root = temp_root("malformed");
        let local_state_dir = root.join(".config/google-chrome");
        fs::create_dir_all(&local_state_dir).unwrap();
        fs::write(local_state_dir.join("Local State"), "not json").unwrap();

        let dirs = Dirs {
            home: root.clone(),
            config_home: root.join(".config"),
            state_home: root.join(".local/state"),
            application_dirs: Vec::new(),
        };
        let browser = native_browser("google-chrome", "/usr/bin/google-chrome-stable %U");
        assert!(profiles_for(&browser, &dirs).is_empty());
        fs::remove_dir_all(&root).ok();
    }
}
