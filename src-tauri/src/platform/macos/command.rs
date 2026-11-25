//! Builds the `open`(1) argv used to launch a browser or app target.
//! `browser.install`'s `exec` holds the `.app` bundle's path (resolved by
//! `browsers.rs`/`apps.rs`), not a shell command — `open -a <bundle>
//! --args <flags...> <url>` is how macOS launches (or re-activates, if
//! already running) an app with argv, without a shell and without
//! needing to parse the bundle's `CFBundleExecutable` ourselves.

use linh_core::{Browser, BrowserFamily, InstallKind, LaunchCommand, Target};

const OPEN: &str = "/usr/bin/open";

fn private_flag(browser: &Browser) -> Option<&'static str> {
    if !browser.supports_private {
        return None;
    }
    match browser.family {
        BrowserFamily::Chromium => Some("--incognito"),
        BrowserFamily::Firefox => Some("-private-window"),
        BrowserFamily::Other => None,
    }
}

pub fn build_launch_command(
    browser: &Browser,
    target: &Target,
    url: &str,
) -> Result<LaunchCommand, String> {
    if url.starts_with('-') {
        return Err("URL must not start with a command-line switch".into());
    }
    let InstallKind::Native { exec } = &browser.install else {
        return Err("macOS does not support Flatpak launches".into());
    };
    if matches!(exec.as_str(), "msteams:" | "zoommtg:") {
        // Teams/Zoom targets: `open` resolves the scheme to whichever
        // app LaunchServices has registered for it, so no bundle path is
        // needed (see `apps::find_app`).
        return Ok(LaunchCommand {
            program: OPEN.to_string(),
            args: vec![url.to_string()],
        });
    }

    let mut app_args = Vec::new();
    if browser.family == BrowserFamily::Chromium {
        if let Some(profile) = &target.profile {
            app_args.push(format!("--profile-directory={profile}"));
        }
    }
    if target.private {
        if let Some(flag) = private_flag(browser) {
            app_args.push(flag.to_string());
        }
    }
    if browser.family == BrowserFamily::Chromium {
        // Guards against the URL being parsed as further flags —
        // everything from here on is the single navigation target.
        app_args.push("--single-argument".to_string());
    }
    app_args.push(url.to_string());

    let mut args = vec!["-a".to_string(), exec.clone(), "--args".to_string()];
    args.extend(app_args);
    Ok(LaunchCommand {
        program: OPEN.to_string(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(id: &str, family: BrowserFamily, supports_private: bool) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family,
            install: InstallKind::Native {
                exec: format!("/Applications/{id}.app"),
            },
            profiles: Vec::new(),
            supports_private,
            icon: None,
            desktop_file: None,
        }
    }

    fn target(profile: Option<&str>, private: bool) -> Target {
        Target {
            browser: "ignored".to_string(),
            profile: profile.map(str::to_string),
            private,
        }
    }

    #[test]
    fn chrome_profile_and_private_place_guard_right_before_url() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, true);
        let cmd = build_launch_command(
            &browser,
            &target(Some("Work"), true),
            "https://example.com/",
        )
        .unwrap();
        assert_eq!(cmd.program, "/usr/bin/open");
        assert_eq!(
            cmd.args,
            vec![
                "-a".to_string(),
                "/Applications/Google Chrome.app".to_string(),
                "--args".to_string(),
                "--profile-directory=Work".to_string(),
                "--incognito".to_string(),
                "--single-argument".to_string(),
                "https://example.com/".to_string(),
            ]
        );
    }

    #[test]
    fn firefox_private_uses_single_dash_flag_and_no_guard() {
        let browser = browser("Firefox", BrowserFamily::Firefox, true);
        let cmd =
            build_launch_command(&browser, &target(None, true), "https://example.com/").unwrap();
        assert_eq!(
            cmd.args,
            vec![
                "-a".to_string(),
                "/Applications/Firefox.app".to_string(),
                "--args".to_string(),
                "-private-window".to_string(),
                "https://example.com/".to_string(),
            ]
        );
    }

    #[test]
    fn safari_has_no_private_flag_even_when_requested() {
        let browser = browser("Safari", BrowserFamily::Other, false);
        let cmd =
            build_launch_command(&browser, &target(None, true), "https://example.com/").unwrap();
        assert_eq!(
            cmd.args,
            vec![
                "-a".to_string(),
                "/Applications/Safari.app".to_string(),
                "--args".to_string(),
                "https://example.com/".to_string(),
            ]
        );
    }

    #[test]
    fn teams_and_zoom_use_open_with_the_url_alone() {
        let browser = Browser {
            id: "teams".to_string(),
            display_name: "teams".to_string(),
            family: BrowserFamily::Other,
            install: InstallKind::Native {
                exec: "msteams:".to_string(),
            },
            profiles: Vec::new(),
            supports_private: false,
            icon: None,
            desktop_file: None,
        };
        let cmd = build_launch_command(
            &browser,
            &target(None, false),
            "msteams://teams.microsoft.com/l/meetup-join/abc",
        )
        .unwrap();
        assert_eq!(cmd.program, "/usr/bin/open");
        assert_eq!(
            cmd.args,
            vec!["msteams://teams.microsoft.com/l/meetup-join/abc".to_string()]
        );
    }

    #[test]
    fn rejects_switch_shaped_url() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, true);
        assert!(build_launch_command(&browser, &target(None, false), "--evil").is_err());
    }

    #[test]
    fn rejects_flatpak_install_kind() {
        let browser = Browser {
            id: "firefox".to_string(),
            display_name: "Firefox".to_string(),
            family: BrowserFamily::Firefox,
            install: InstallKind::Flatpak {
                app_id: "org.mozilla.firefox".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: None,
        };
        assert!(
            build_launch_command(&browser, &target(None, false), "https://example.com/").is_err()
        );
    }
}
