use std::collections::HashMap;
use std::fs;

use linh_core::{Browser, BrowserFamily, InstallKind, Target};

use super::desktop::{self, DesktopEntry};
use super::dirs::Dirs;
use super::logging::log_line;
use super::LINH_DESKTOP_ID;

const CHROMIUM_KEYWORDS: [&str; 5] = ["chrome", "chromium", "brave", "edge", "vivaldi"];
const FIREFOX_KEYWORDS: [&str; 3] = ["firefox", "librewolf", "zen"];

/// Scans the `.desktop` directories in `dirs` for installed http(s)
/// browsers, applying override precedence (a later directory wins on a
/// duplicate desktop id, including a `Hidden=true` entry with no `Exec`
/// at all acting as a tombstone that removes a lower-priority entry),
/// hiding `Hidden=true` entries and Linh itself, keeping only entries
/// that handle `x-scheme-handler/http(s)`, and excluding (with a log
/// line) any browser — native or Flatpak — whose `Exec=` line
/// `linh-core` rejects.
pub fn scan_browsers(dirs: &Dirs) -> Vec<Browser> {
    let mut by_id: HashMap<String, DesktopEntry> = HashMap::new();

    for dir in &dirs.application_dirs {
        let Ok(read_dir) = fs::read_dir(dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(parsed) = desktop::parse_desktop_file(&path, id) {
                by_id.insert(id.to_string(), parsed);
            }
        }
    }

    let mut browsers: Vec<Browser> = by_id
        .into_values()
        .filter_map(|entry| build_browser(entry, dirs))
        .collect();
    browsers.sort_by(|a, b| a.id.cmp(&b.id));
    browsers
}

fn build_browser(entry: DesktopEntry, dirs: &Dirs) -> Option<Browser> {
    if entry.hidden || is_self(&entry.id) {
        return None;
    }
    let handles_http = entry
        .mime_types
        .iter()
        .any(|m| m == "x-scheme-handler/http" || m == "x-scheme-handler/https");
    if !handles_http {
        return None;
    }

    // Validate the raw `Exec=` line structurally for *every* entry,
    // native or Flatpak: an empty, unterminated-quote, or otherwise
    // `linh-core`-rejected Exec excludes the browser either way, even
    // though a Flatpak browser is ultimately launched via `flatpak run`
    // rather than this Exec line.
    if let Err(err) = validate_exec(&entry) {
        log_line(
            dirs,
            &format!("excluding browser '{}': invalid Exec: {err}", entry.id),
        );
        return None;
    }

    let install = resolve_install_kind(&entry)?;
    let family = classify_family(&entry.id, &entry.exec);

    Some(Browser {
        id: entry.id.clone(),
        display_name: entry.name.clone(),
        family,
        install,
        profiles: Vec::new(),
        supports_private: matches!(family, BrowserFamily::Chromium | BrowserFamily::Firefox),
        icon: entry.icon.clone(),
        desktop_file: Some(entry.path.to_string_lossy().into_owned()),
    })
}

/// Runs `entry.exec` through `linh-core`'s native-Exec parser purely to
/// validate it (empty, unmatched quote, bad field code, ...); the probe
/// browser's `InstallKind` is always `Native` here regardless of how
/// `entry` actually resolves, since the validation is about the Exec
/// line's own syntax, not about how the browser is ultimately launched.
fn validate_exec(entry: &DesktopEntry) -> Result<(), linh_core::LaunchError> {
    let probe = Browser {
        id: entry.id.clone(),
        display_name: entry.name.clone(),
        family: BrowserFamily::Other,
        install: InstallKind::Native {
            exec: entry.exec.clone(),
        },
        profiles: Vec::new(),
        supports_private: false,
        icon: entry.icon.clone(),
        desktop_file: Some(entry.path.to_string_lossy().into_owned()),
    };
    let probe_target = Target {
        browser: entry.id.clone(),
        profile: None,
        private: false,
    };
    linh_core::build_launch_command(&probe, &probe_target, "https://example.com").map(|_| ())
}

fn resolve_install_kind(entry: &DesktopEntry) -> Option<InstallKind> {
    if let Some(app_id) = &entry.flatpak_app_id {
        if !app_id.is_empty() {
            return Some(InstallKind::Flatpak {
                app_id: app_id.clone(),
            });
        }
    }
    if let Some(app_id) = desktop::flatpak_app_id_from_exec(&entry.exec) {
        return Some(InstallKind::Flatpak { app_id });
    }
    Some(InstallKind::Native {
        exec: entry.exec.clone(),
    })
}

fn classify_family(id: &str, exec: &str) -> BrowserFamily {
    let haystack = format!("{} {}", id.to_lowercase(), exec.to_lowercase());
    if CHROMIUM_KEYWORDS.iter().any(|k| haystack.contains(k)) {
        BrowserFamily::Chromium
    } else if FIREFOX_KEYWORDS.iter().any(|k| haystack.contains(k)) {
        BrowserFamily::Firefox
    } else {
        BrowserFamily::Other
    }
}

fn is_self(id: &str) -> bool {
    id.eq_ignore_ascii_case(LINH_DESKTOP_ID) || id.eq_ignore_ascii_case("dev.tkhieu.linh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_desktop(dir: &std::path::Path, name: &str, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), content).unwrap();
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-scan-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// A `Dirs` whose `application_dirs` is exactly `dirs_to_scan`, in
    /// order (so the last one wins on a duplicate desktop id) — never the
    /// real `/usr/share/applications` or `$HOME`.
    fn test_dirs(root: &std::path::Path, dirs_to_scan: &[PathBuf]) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: dirs_to_scan.to_vec(),
        }
    }

    #[test]
    fn native_browser_detected_with_http_mimetype() {
        let root = temp_root("native");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "google-chrome.desktop",
            "[Desktop Entry]\nName=Google Chrome\nExec=/usr/bin/google-chrome-stable %U\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].id, "google-chrome");
        assert_eq!(browsers[0].family, BrowserFamily::Chromium);
        assert!(matches!(browsers[0].install, InstallKind::Native { .. }));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_http_handler_is_skipped() {
        let root = temp_root("nonhttp");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "editor.desktop",
            "[Desktop Entry]\nName=Editor\nExec=/usr/bin/editor %f\nMimeType=text/plain;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hidden_entry_is_skipped() {
        let root = temp_root("hidden");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "chrome.desktop",
            "[Desktop Entry]\nName=Chrome\nExec=/usr/bin/chrome %u\nHidden=true\nMimeType=x-scheme-handler/http;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn self_desktop_entry_is_skipped() {
        let root = temp_root("self");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "Linh.desktop",
            "[Desktop Entry]\nName=Linh\nExec=linh %u\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn invalid_exec_is_excluded() {
        let root = temp_root("invalid-exec");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "broken.desktop",
            "[Desktop Entry]\nName=Broken\nExec=\nMimeType=x-scheme-handler/http;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn user_applications_entry_overrides_system_flatpak_entry() {
        let root = temp_root("user-overrides-system-flatpak");
        let system_flatpak = root.join("system-flatpak");
        let user_apps = root.join("user-apps");
        write_desktop(
            &system_flatpak,
            "org.mozilla.firefox.desktop",
            "[Desktop Entry]\nName=Firefox (system flatpak)\nExec=/usr/bin/flatpak run org.mozilla.firefox %u\nX-Flatpak=org.mozilla.firefox\nMimeType=x-scheme-handler/http;\n",
        );
        write_desktop(
            &user_apps,
            "org.mozilla.firefox.desktop",
            "[Desktop Entry]\nName=Firefox (user native)\nExec=/usr/bin/firefox %u\nMimeType=x-scheme-handler/http;\n",
        );
        // user_apps is listed after system_flatpak, so it must win.
        let browsers = scan_browsers(&test_dirs(&root, &[system_flatpak, user_apps]));
        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].display_name, "Firefox (user native)");
        assert!(matches!(browsers[0].install, InstallKind::Native { .. }));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hidden_tombstone_without_exec_removes_lower_priority_entry() {
        let root = temp_root("hidden-tombstone");
        let system_dir = root.join("system");
        let user_dir = root.join("user");
        write_desktop(
            &system_dir,
            "chrome.desktop",
            "[Desktop Entry]\nName=Chrome\nExec=/usr/bin/chrome %u\nMimeType=x-scheme-handler/http;\n",
        );
        // Higher-priority entry: Hidden=true, no Exec at all.
        write_desktop(
            &user_dir,
            "chrome.desktop",
            "[Desktop Entry]\nName=Chrome\nHidden=true\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[system_dir, user_dir]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_flatpak_exec_is_excluded() {
        let root = temp_root("malformed-flatpak-exec");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "org.mozilla.firefox.desktop",
            "[Desktop Entry]\nName=Firefox\nExec=\nX-Flatpak=org.mozilla.firefox\nMimeType=x-scheme-handler/http;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn flatpak_exec_with_unmatched_quote_is_excluded() {
        let root = temp_root("flatpak-unmatched-quote");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "org.mozilla.firefox.desktop",
            "[Desktop Entry]\nName=Firefox\nExec=/usr/bin/flatpak run \"org.mozilla.firefox %u\nX-Flatpak=org.mozilla.firefox\nMimeType=x-scheme-handler/http;\n",
        );
        let browsers = scan_browsers(&test_dirs(&root, &[apps]));
        assert!(browsers.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn later_dir_overrides_same_desktop_id() {
        let root = temp_root("override");
        let apps1 = root.join("apps1");
        let apps2 = root.join("apps2");
        write_desktop(
            &apps1,
            "firefox.desktop",
            "[Desktop Entry]\nName=Firefox (native)\nExec=/usr/bin/firefox %u\nMimeType=x-scheme-handler/http;\n",
        );
        write_desktop(
            &apps2,
            "firefox.desktop",
            "[Desktop Entry]\nName=Firefox (flatpak)\nExec=/usr/bin/flatpak run org.mozilla.firefox %u\nX-Flatpak=org.mozilla.firefox\nMimeType=x-scheme-handler/http;\n",
        );
        // apps2 is listed after apps1, so it should win.
        let browsers = scan_browsers(&test_dirs(&root, &[apps1, apps2]));
        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].display_name, "Firefox (flatpak)");
        assert!(matches!(browsers[0].install, InstallKind::Flatpak { .. }));
        fs::remove_dir_all(&root).ok();
    }
}
