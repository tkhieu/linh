//! Detection of the non-browser desktop apps an `AppTarget` rule can
//! launch (Teams, Zoom), via the `x-scheme-handler/...` MIME type each
//! declares in its `.desktop` file. These are never browsers and must
//! never appear in `list_browsers`' result.

use linh_core::Browser;

use super::dirs::Dirs;
use super::scan;

/// The MIME type that identifies an installed desktop app capable of
/// handling `app_id` ("teams" | "zoom"), per the plan's Contract
/// section. An unrecognized `app_id` has no handler MIME and is
/// therefore never "installed".
fn handler_mime(app_id: &str) -> Option<&'static str> {
    match app_id {
        "teams" => Some("x-scheme-handler/msteams"),
        "zoom" => Some("x-scheme-handler/zoommtg"),
        _ => None,
    }
}

/// Finds the installed desktop app (if any) declaring the handler MIME
/// type for `app_id`, using the same `.desktop` scan, override
/// precedence, and `Exec=` validation as browser detection. Returned as
/// a `Browser` purely so `linh_core::build_launch_command` (which takes
/// one) can be reused to build its launch argv.
pub fn find_app(dirs: &Dirs, app_id: &str) -> Option<Browser> {
    let mime = handler_mime(app_id)?;
    scan::scan_entries_for_mime(dirs, mime).into_iter().next()
}

/// Whether an app satisfying `app_id`'s handler MIME type is installed.
pub fn is_app_installed(dirs: &Dirs, app_id: &str) -> bool {
    find_app(dirs, app_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write_desktop(dir: &std::path::Path, name: &str, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), content).unwrap();
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-apps-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_dirs(root: &std::path::Path, dirs_to_scan: &[PathBuf]) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: dirs_to_scan.to_vec(),
        }
    }

    #[test]
    fn finds_teams_for_linux_by_msteams_handler() {
        let root = temp_root("teams");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "com.github.IsmaelMartinez.teams_for_linux.desktop",
            "[Desktop Entry]\nName=Teams for Linux\nExec=/usr/bin/teams-for-linux %U\nMimeType=x-scheme-handler/msteams;\n",
        );
        let dirs = test_dirs(&root, &[apps]);
        assert!(is_app_installed(&dirs, "teams"));
        assert_eq!(
            find_app(&dirs, "teams").unwrap().display_name,
            "Teams for Linux"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finds_zoom_by_zoommtg_handler() {
        let root = temp_root("zoom");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "Zoom.desktop",
            "[Desktop Entry]\nName=Zoom\nExec=/usr/bin/zoom %U\nMimeType=x-scheme-handler/zoommtg;\n",
        );
        let dirs = test_dirs(&root, &[apps]);
        assert!(is_app_installed(&dirs, "zoom"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn teams_not_installed_when_no_matching_handler() {
        let root = temp_root("none");
        let dirs = test_dirs(&root, &[]);
        assert!(!is_app_installed(&dirs, "teams"));
        assert!(!is_app_installed(&dirs, "zoom"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_app_id_is_never_installed() {
        let root = temp_root("unknown");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "google-chrome.desktop",
            "[Desktop Entry]\nName=Google Chrome\nExec=/usr/bin/google-chrome-stable %U\nMimeType=x-scheme-handler/http;\n",
        );
        let dirs = test_dirs(&root, &[apps]);
        assert!(!is_app_installed(&dirs, "something-else"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn zoom_handler_does_not_count_as_teams_installed() {
        let root = temp_root("cross");
        let apps = root.join("apps");
        write_desktop(
            &apps,
            "Zoom.desktop",
            "[Desktop Entry]\nName=Zoom\nExec=/usr/bin/zoom %U\nMimeType=x-scheme-handler/zoommtg;\n",
        );
        let dirs = test_dirs(&root, &[apps]);
        assert!(!is_app_installed(&dirs, "teams"));
        fs::remove_dir_all(&root).ok();
    }
}
