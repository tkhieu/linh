use std::process::Command;

use linh_core::Browser;

use super::LINH_DESKTOP_ID;

fn desktop_id() -> String {
    format!("{LINH_DESKTOP_ID}.desktop")
}

/// Runs `xdg-settings get default-web-browser`, returning the desktop id
/// it reports, if any.
pub fn current_default_browser_id() -> Option<String> {
    let output = Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Whether `xdg-settings` currently reports Linh as the default browser.
pub fn is_default_browser() -> bool {
    current_default_browser_id()
        .map(|id| id.eq_ignore_ascii_case(&desktop_id()))
        .unwrap_or(false)
}

/// Registers Linh as the default browser for `http`/`https`/`text/html`
/// via `xdg-settings` and `xdg-mime`, then re-checks the result.
pub fn set_default_browser() -> Result<(), String> {
    run(
        "xdg-settings",
        &["set", "default-web-browser", &desktop_id()],
    )?;
    run(
        "xdg-mime",
        &[
            "default",
            &desktop_id(),
            "x-scheme-handler/http",
            "x-scheme-handler/https",
            "text/html",
        ],
    )?;
    if is_default_browser() {
        Ok(())
    } else {
        Err("xdg-settings did not confirm Linh as the default browser".to_string())
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run {cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} {args:?} exited with {status}"))
    }
}

/// True if `xdg_id` (as reported by `xdg-settings`) refers to `browser`
/// (i.e. `<browser.id>.desktop`, or the basename of its known desktop
/// file).
pub fn matches_desktop_id(browser: &Browser, xdg_id: &str) -> bool {
    if format!("{}.desktop", browser.id).eq_ignore_ascii_case(xdg_id) {
        return true;
    }
    if let Some(desktop_file) = &browser.desktop_file {
        if let Some(basename) = std::path::Path::new(desktop_file)
            .file_name()
            .and_then(|f| f.to_str())
        {
            return basename.eq_ignore_ascii_case(xdg_id);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{BrowserFamily, InstallKind};

    fn browser(id: &str, desktop_file: Option<&str>) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: "/usr/bin/x %u".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: desktop_file.map(str::to_string),
        }
    }

    #[test]
    fn matches_by_id_plus_desktop_suffix() {
        assert!(matches_desktop_id(
            &browser("google-chrome", None),
            "google-chrome.desktop"
        ));
    }

    #[test]
    fn matches_by_desktop_file_basename() {
        assert!(matches_desktop_id(
            &browser(
                "chrome",
                Some("/var/lib/flatpak/exports/share/applications/org.chromium.Chromium.desktop")
            ),
            "org.chromium.Chromium.desktop"
        ));
    }

    #[test]
    fn does_not_match_unrelated_id() {
        assert!(!matches_desktop_id(
            &browser("firefox", Some("/usr/share/applications/firefox.desktop")),
            "google-chrome.desktop"
        ));
    }
}
