use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use linh_core::{Browser, BrowserFamily, InstallKind, LaunchCommand, Target};

use crate::platform::logging::log_line;

use super::chromium;
use super::desktop::{self, DesktopEntry};
use super::dirs::Dirs;
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
#[cfg(test)]
pub fn scan_browsers(dirs: &Dirs) -> Vec<Browser> {
    scan_browser_discovery(dirs).browsers
}

/// Browser ids dropped because another desktop entry launches the same install.
pub struct BrowserDiscovery {
    pub browsers: Vec<Browser>,
    pub aliases: HashMap<String, String>,
}

pub fn scan_browser_discovery(dirs: &Dirs) -> BrowserDiscovery {
    let entries = collect_entries(dirs);
    let mut groups: HashMap<EquivalenceKey, Vec<Candidate>> = HashMap::new();
    for entry in entries.into_values().filter(|entry| {
        !entry.hidden
            && !is_self(&entry.id)
            && entry
                .mime_types
                .iter()
                .any(|m| m == "x-scheme-handler/http" || m == "x-scheme-handler/https")
    }) {
        let Some(browser) = finish_entry(entry.clone(), dirs) else {
            continue;
        };
        let Some(command) = identity_command(&entry) else {
            continue;
        };
        let key = equivalence_key(&browser, &command, dirs);
        let dir = entry.path.parent().unwrap_or(Path::new(""));
        let scan_order = dirs
            .application_dirs
            .iter()
            .position(|p| p == dir)
            .unwrap_or(usize::MAX);
        // The first three resolved dirs are system locations; later dirs
        // include XDG_DATA_HOME even when it is outside HOME.
        let user_dir = dir.starts_with(&dirs.home) || scan_order >= 3;
        groups.entry(key).or_default().push(Candidate {
            browser,
            no_display: entry.no_display,
            user_dir,
            scan_order,
        });
    }
    let mut browsers = Vec::new();
    let mut aliases = HashMap::new();
    for mut group in groups.into_values() {
        group.sort_by(|a, b| {
            a.no_display
                .cmp(&b.no_display)
                .then_with(|| b.user_dir.cmp(&a.user_dir))
                // A later directory is the stronger XDG override, as in
                // collect_entries for entries with the same desktop id.
                .then_with(|| b.scan_order.cmp(&a.scan_order))
                .then_with(|| a.browser.id.cmp(&b.browser.id))
        });
        let winner = group.remove(0).browser;
        for dropped in group {
            aliases.insert(dropped.browser.id, winner.id.clone());
        }
        browsers.push(winner);
    }
    browsers.sort_by(|a, b| a.id.cmp(&b.id));
    BrowserDiscovery { browsers, aliases }
}

/// Scans the same `.desktop` directories, with the same override
/// precedence, hidden/self exclusion, and `Exec=` validation as
/// [`scan_browsers`], for entries that declare `mime` (e.g.
/// `x-scheme-handler/msteams`). Used for detecting non-browser desktop
/// apps (Teams, Zoom) that Linh can hand a URL to; the result is a
/// `Browser` value purely so `linh-core::build_launch_command` (which
/// takes one) can be reused for launching them — these are never web
/// browsers and must never be surfaced via [`scan_browsers`] / Linh's
/// `list_browsers` command.
pub fn scan_entries_for_mime(dirs: &Dirs, mime: &str) -> Vec<Browser> {
    scan_entries(dirs, |entry| entry.mime_types.iter().any(|m| m == mime))
}

fn scan_entries(dirs: &Dirs, handles: impl Fn(&DesktopEntry) -> bool) -> Vec<Browser> {
    let mut entries: Vec<Browser> = collect_entries(dirs)
        .into_values()
        .filter(|entry| !entry.hidden && !is_self(&entry.id) && handles(entry))
        .filter_map(|entry| finish_entry(entry, dirs))
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

fn collect_entries(dirs: &Dirs) -> HashMap<String, DesktopEntry> {
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
            // UI target keys reserve '/' and '#private' as separators.
            if id.contains(['/', '#']) {
                continue;
            }
            if let Some(parsed) = desktop::parse_desktop_file(&path, id) {
                by_id.insert(id.to_string(), parsed);
            }
        }
    }

    by_id
}

struct Candidate {
    browser: Browser,
    no_display: bool,
    user_dir: bool,
    scan_order: usize,
}

#[derive(Hash, PartialEq, Eq)]
enum EquivalenceKey {
    Native(PathBuf, Option<PathBuf>, Vec<String>),
    Flatpak(String, PathBuf, Vec<String>),
}

const IDENTITY_URL: &str = "https://linh.invalid/identity";

fn identity_command(entry: &DesktopEntry) -> Option<LaunchCommand> {
    let probe = Browser {
        id: entry.id.clone(),
        display_name: String::new(),
        family: BrowserFamily::Other,
        install: InstallKind::Native {
            exec: entry.exec.clone(),
        },
        profiles: Vec::new(),
        supports_private: false,
        icon: None,
        desktop_file: None,
    };
    linh_core::build_launch_command(
        &probe,
        &Target {
            browser: entry.id.clone(),
            profile: None,
            private: false,
        },
        IDENTITY_URL,
    )
    .ok()
}

fn canonical_program(program: &str) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') {
        return fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    }
    if let Some(found) = env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
    {
        return fs::canonicalize(&found).unwrap_or(found);
    }
    path.to_path_buf()
}

fn equivalence_key(browser: &Browser, command: &LaunchCommand, dirs: &Dirs) -> EquivalenceKey {
    let args: Vec<String> = command
        .args
        .iter()
        .filter(|arg| arg.as_str() != IDENTITY_URL)
        .cloned()
        .collect();
    match &browser.install {
        InstallKind::Native { .. } => {
            let user_data = args
                .iter()
                .find_map(|arg| arg.strip_prefix("--user-data-dir="))
                .map(PathBuf::from)
                .or_else(|| {
                    args.windows(2)
                        .find(|pair| pair[0] == "--user-data-dir")
                        .map(|pair| PathBuf::from(&pair[1]))
                });
            let profile_dir = if browser.family == BrowserFamily::Chromium {
                user_data.or_else(|| {
                    chromium::local_state_path(browser, dirs)
                        .and_then(|p| p.parent().map(Path::to_path_buf))
                })
            } else {
                None
            };
            EquivalenceKey::Native(canonical_program(&command.program), profile_dir, args)
        }
        InstallKind::Flatpak { app_id } => EquivalenceKey::Flatpak(
            app_id.clone(),
            dirs.home.join(".var/app").join(app_id),
            args,
        ),
    }
}

fn finish_entry(entry: DesktopEntry, dirs: &Dirs) -> Option<Browser> {
    // Validate the raw `Exec=` line structurally for *every* entry,
    // native or Flatpak: an empty, unterminated-quote, or otherwise
    // `linh-core`-rejected Exec excludes it either way, even though a
    // Flatpak entry is ultimately launched via `flatpak run` rather than
    // this Exec line.
    if let Err(err) = validate_exec(&entry) {
        log_line(
            dirs,
            &format!("excluding '{}': invalid Exec: {err}", entry.id),
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

/// True for the native desktop id (`linh`) or either of Linh's Flatpak
/// app ids: the current `dev.hieutk.linh` and the pre-rename
/// `dev.tkhieu.linh` (Flathub requires the app id to match a domain the
/// owner controls, hence the rename), so a Flatpak install under either
/// id is never listed as a separate, installable browser.
fn is_self(id: &str) -> bool {
    id.eq_ignore_ascii_case(LINH_DESKTOP_ID)
        || id.eq_ignore_ascii_case("dev.hieutk.linh")
        || id.eq_ignore_ascii_case("dev.tkhieu.linh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn chrome_entry(exec: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nName=Chrome\nExec={exec}\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n{extra}")
    }

    #[test]
    fn equivalent_native_launchers_keep_visible_and_record_alias() {
        let root = temp_root("equivalent");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "google-chrome.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        write_desktop(
            &apps,
            "com.google.Chrome.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", "NoDisplay=true\n"),
        );
        let found = scan_browser_discovery(&test_dirs(&root, &[apps]));
        assert_eq!(found.browsers.len(), 1);
        assert_eq!(found.browsers[0].id, "google-chrome");
        assert_eq!(
            found.aliases.get("com.google.Chrome").map(String::as_str),
            Some("google-chrome")
        );
        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_exec_is_equivalent() {
        let root = temp_root("symlink");
        let apps = root.join("apps");
        let binary = root.join("chrome-bin");
        let link = root.join("chrome-link");
        fs::write(&binary, "").unwrap();
        std::os::unix::fs::symlink(&binary, &link).unwrap();
        write_desktop(
            &apps,
            "visible.desktop",
            &chrome_entry(&format!("{} %U", binary.display()), ""),
        );
        write_desktop(
            &apps,
            "hidden.desktop",
            &chrome_entry(&format!("{} %U", link.display()), "NoDisplay=true\n"),
        );
        let found = scan_browser_discovery(&test_dirs(&root, &[apps]));
        assert_eq!(found.browsers.len(), 1);
        assert_eq!(
            found.aliases.get("hidden").map(String::as_str),
            Some("visible")
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn native_flatpak_beta_and_user_data_are_distinct() {
        let root = temp_root("distinct");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "google-chrome.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        write_desktop(
            &apps,
            "flatpak-chrome.desktop",
            &chrome_entry(
                "/usr/bin/flatpak run com.google.Chrome %U",
                "X-Flatpak=com.google.Chrome\n",
            ),
        );
        write_desktop(
            &apps,
            "google-chrome-beta.desktop",
            &chrome_entry("/usr/bin/google-chrome-beta %U", ""),
        );
        write_desktop(
            &apps,
            "custom-chrome.desktop",
            &chrome_entry(
                "/usr/bin/google-chrome-stable --user-data-dir=/tmp/chrome-custom %U",
                "",
            ),
        );
        let found = scan_browser_discovery(&test_dirs(&root, &[apps]));
        assert_eq!(found.browsers.len(), 4);
        assert!(found.aliases.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn user_dir_beats_earlier_system_dir_for_equivalent_ids() {
        let root = temp_root("user-priority");
        let system = root.join("system");
        let user = root.join("home/.local/share/applications");
        write_desktop(
            &system,
            "system.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        write_desktop(
            &user,
            "user.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        let found = scan_browser_discovery(&test_dirs(&root, &[system, user]));
        assert_eq!(found.browsers.len(), 1);
        assert_eq!(found.browsers[0].id, "user");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn custom_xdg_user_dir_outside_home_is_preferred() {
        let root = temp_root("custom-xdg-priority");
        let system = root.join("system");
        let custom = root.join("custom-data/applications");
        write_desktop(
            &system,
            "system.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        write_desktop(
            &custom,
            "custom.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        let found = scan_browser_discovery(&test_dirs(
            &root,
            &[system, root.join("system2"), root.join("system3"), custom],
        ));
        assert_eq!(found.browsers.len(), 1);
        assert_eq!(found.browsers[0].id, "custom");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn later_equal_tier_directory_wins() {
        let root = temp_root("equal-tier-order");
        let first = root.join("system-a");
        let later = root.join("system-b");
        write_desktop(
            &first,
            "a.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        write_desktop(
            &later,
            "z.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        let found = scan_browser_discovery(&test_dirs(&root, &[first, later]));
        assert_eq!(found.browsers.len(), 1);
        assert_eq!(found.browsers[0].id, "z");
        assert_eq!(found.aliases.get("a").map(String::as_str), Some("z"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn desktop_id_with_target_key_separator_is_skipped() {
        let root = temp_root("invalid-id-separator");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "chrome#extra.desktop",
            &chrome_entry("/usr/bin/google-chrome-stable %U", ""),
        );
        assert!(scan_browsers(&test_dirs(&root, &[apps])).is_empty());
        fs::remove_dir_all(&root).ok();
    }

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
    fn is_self_matches_native_id_and_both_flatpak_app_ids() {
        assert!(is_self(LINH_DESKTOP_ID));
        assert!(is_self("dev.hieutk.linh"));
        assert!(is_self("dev.tkhieu.linh"));
        assert!(is_self("DEV.HIEUTK.LINH"));
        assert!(!is_self("org.mozilla.firefox"));
    }

    #[test]
    fn self_desktop_entry_is_skipped() {
        let root = temp_root("self");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "linh.desktop",
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

    #[test]
    fn scan_entries_for_mime_finds_non_browser_app() {
        let root = temp_root("mime-app");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "com.github.IsmaelMartinez.teams_for_linux.desktop",
            "[Desktop Entry]\nName=Teams for Linux\nExec=/usr/bin/teams-for-linux %U\nMimeType=x-scheme-handler/msteams;\n",
        );
        let apps_found =
            scan_entries_for_mime(&test_dirs(&root, &[apps]), "x-scheme-handler/msteams");
        assert_eq!(apps_found.len(), 1);
        assert_eq!(apps_found[0].display_name, "Teams for Linux");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_entries_for_mime_ignores_non_matching_mime() {
        let root = temp_root("mime-app-nomatch");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "google-chrome.desktop",
            "[Desktop Entry]\nName=Google Chrome\nExec=/usr/bin/google-chrome-stable %U\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        );
        let apps_found =
            scan_entries_for_mime(&test_dirs(&root, &[apps]), "x-scheme-handler/msteams");
        assert!(apps_found.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_entries_for_mime_respects_hidden_and_self_exclusion() {
        let root = temp_root("mime-app-hidden-self");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "hidden-teams.desktop",
            "[Desktop Entry]\nName=Hidden Teams\nExec=/usr/bin/teams %U\nHidden=true\nMimeType=x-scheme-handler/msteams;\n",
        );
        write_desktop(
            &apps,
            "linh.desktop",
            "[Desktop Entry]\nName=Linh\nExec=linh %u\nMimeType=x-scheme-handler/msteams;\n",
        );
        let apps_found =
            scan_entries_for_mime(&test_dirs(&root, &[apps]), "x-scheme-handler/msteams");
        assert!(apps_found.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_entries_for_mime_finds_flatpak_teams_with_double_at_url_syntax() {
        // Real Flatpak exports for URL-handling apps can emit an Exec
        // line like `... @@u %U @@`: the `@@u`/`@@` tokens are literal
        // (not field codes) and, since the entry resolves to a Flatpak
        // install, are never even consulted at launch time (launching a
        // Flatpak app only ever runs `flatpak run <app-id> ...`) — but
        // the Exec line must still parse as valid so the entry isn't
        // excluded, and the app id must still be extracted correctly.
        let root = temp_root("mime-app-flatpak-atat");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "com.github.IsmaelMartinez.teams_for_linux.desktop",
            "[Desktop Entry]\nName=Teams for Linux\nExec=/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=teams-for-linux com.github.IsmaelMartinez.teams_for_linux @@u %U @@\nMimeType=x-scheme-handler/msteams;\n",
        );
        let apps_found =
            scan_entries_for_mime(&test_dirs(&root, &[apps]), "x-scheme-handler/msteams");
        assert_eq!(apps_found.len(), 1);
        assert_eq!(
            apps_found[0].install,
            InstallKind::Flatpak {
                app_id: "com.github.IsmaelMartinez.teams_for_linux".to_string()
            }
        );
        fs::remove_dir_all(&root).ok();
    }
}
