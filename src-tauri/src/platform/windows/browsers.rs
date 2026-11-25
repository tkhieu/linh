use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use linh_core::{Browser, BrowserFamily, BrowserProfile, InstallKind};
use winreg::enums::KEY_READ;
use winreg::RegKey;

use super::command::parse_registry_command;
use super::dirs::Dirs;
use super::registry::{current_user, local_machine, read_string};

const CLIENTS: [&str; 2] = [
    r"SOFTWARE\Clients\StartMenuInternet",
    r"SOFTWARE\WOW6432Node\Clients\StartMenuInternet",
];

fn classify(name: &str) -> (BrowserFamily, bool) {
    let name = name.to_ascii_lowercase();
    if name.contains("firefox") {
        (BrowserFamily::Firefox, true)
    } else if ["chrome", "chromium", "edge", "brave", "vivaldi", "opera"]
        .iter()
        .any(|s| name.contains(s))
    {
        (BrowserFamily::Chromium, true)
    } else {
        (BrowserFamily::Other, false)
    }
}

fn profile_root(id: &str, dirs: &Dirs) -> Option<PathBuf> {
    let id = id.to_ascii_lowercase();
    let base = if id.contains("opera") {
        &dirs.config_home
    } else {
        &dirs.state_home
    };
    let subpath = if id.contains("chrome") {
        "Google/Chrome/User Data"
    } else if id.contains("edge") {
        "Microsoft/Edge/User Data"
    } else if id.contains("brave") {
        "BraveSoftware/Brave-Browser/User Data"
    } else if id.contains("vivaldi") {
        "Vivaldi/User Data"
    } else if id.contains("chromium") {
        "Chromium/User Data"
    } else if id.contains("opera") {
        "Opera Software/Opera Stable"
    } else {
        return None;
    };
    Some(base.parent()?.join(subpath))
}

fn read_profiles(path: &Path) -> Vec<BrowserProfile> {
    let Ok(data) = fs::read_to_string(path.join("Local State")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) else {
        return Vec::new();
    };
    let Some(cache) = json
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };
    let mut profiles: Vec<_> = cache
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

fn scan_root(root: &RegKey, prefix: &str, dirs: &Dirs, results: &mut BTreeMap<String, Browser>) {
    let Ok(clients) = root.open_subkey_with_flags(prefix, KEY_READ) else {
        return;
    };
    for key in clients.enum_keys().flatten() {
        if key.eq_ignore_ascii_case("linh") {
            continue;
        }
        let path = format!(r"{prefix}\{key}");
        let Some(command) = read_string(root, &format!(r"{path}\shell\open\command"), "") else {
            continue;
        };
        let Some((exe, _)) = parse_registry_command(&command) else {
            continue;
        };
        if !Path::new(&exe).is_file() {
            continue;
        }
        let display_name = read_string(root, &path, "")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| key.clone());
        let (family, supports_private) = classify(&format!("{key} {display_name}"));
        let icon = read_string(root, &format!(r"{path}\DefaultIcon"), "").or(Some(exe));
        let mut browser = Browser {
            id: key.to_ascii_lowercase(),
            display_name,
            family,
            install: InstallKind::Native { exec: command },
            profiles: Vec::new(),
            supports_private,
            icon,
            desktop_file: None,
        };
        if family == BrowserFamily::Chromium && !key.to_ascii_lowercase().contains("opera") {
            if let Some(root) = profile_root(&key, dirs) {
                browser.profiles = read_profiles(&root);
            }
        }
        results.insert(browser.id.clone(), browser);
    }
}

pub fn detect_browsers(dirs: &Dirs) -> Vec<Browser> {
    let mut results = BTreeMap::new();
    for prefix in CLIENTS {
        scan_root(&local_machine(), prefix, dirs, &mut results);
    }
    for prefix in CLIENTS {
        scan_root(&current_user(), prefix, dirs, &mut results);
    }
    results.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_browser_families() {
        for name in [
            "Google Chrome",
            "Microsoft Edge",
            "Brave",
            "Vivaldi",
            "Opera",
        ] {
            assert_eq!(classify(name), (BrowserFamily::Chromium, true));
        }
        assert_eq!(classify("Mozilla Firefox"), (BrowserFamily::Firefox, true));
        assert_eq!(classify("Some Browser"), (BrowserFamily::Other, false));
    }
}
