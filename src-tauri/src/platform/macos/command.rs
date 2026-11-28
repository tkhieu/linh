//! Builds the `open`(1) argv used to launch a browser or app target.
//! `browser.install`'s `exec` holds the `.app` bundle's path (resolved by
//! `browsers.rs`/`apps.rs`), not a shell command.
//!
//! Delivery to an *already-running* instance depends on whether any
//! extra flags are needed:
//!
//! - No profile/private flag: `open -a <bundle> <url>`, with the URL as
//!   a plain positional argument (never after `--args`). `open`(1) hands
//!   a positional URL/file argument to LaunchServices as something to
//!   open *with* the target app, which is delivered the same way (an
//!   Apple Event) whether or not the app is already running. This is
//!   also how Teams/Zoom targets are opened (just `open <scheme-url>`,
//!   no `-a`): see `apps::find_app`.
//! - A profile or private-window flag: `open`(1)'s own man page says
//!   `--args` passes its remaining arguments to "the new instance of
//!   the application" — plain `open -a` only *activates* an
//!   already-running one rather than starting a new instance, so those
//!   arguments would never reach it (`open` still exits 0, having just
//!   brought the existing window forward with no URL, no profile, no
//!   private window — a widely-reported real-world gotcha, e.g.
//!   <https://forum.vivaldi.net/post/570354>, Mozilla support thread
//!   <https://support.mozilla.org/bm/questions/1375699>). `open`'s `-n`
//!   flag ("open a new instance of the application(s) even if one is
//!   already running") forces that new-instance launch, which is the
//!   documented, widely-used fix for this exact problem for both Chrome
//!   and Firefox profile/incognito flags, e.g.
//!   <https://gist.github.com/gm3dmo/98247152b375b84ac7a9d4cbb7f92e3b>
//!   (`open -na "Google Chrome" --args --profile-directory=...`) and
//!   Firefox's own remoting service
//!   (`toolkit/components/remote/nsMacRemoteClient.mm`'s
//!   `SendCommandLine`, which does forward a new instance's full argv,
//!   including `-private-window`, to a running one over a Mach port).

use linh_core::{Browser, BrowserFamily, InstallKind, LaunchCommand, Target};

use crate::updates::{current_dist, Dist};

/// Also referenced by `platform::launcher::spawn_detached`, to apply its
/// bounded-wait handling only to an `open`-wrapped `LaunchCommand` (every
/// non-MAS one this module builds is).
pub(crate) const OPEN: &str = "/usr/bin/open";

/// Sentinel `LaunchCommand::program` values this module hands back
/// instead of `OPEN` when `current_dist()` is [`Dist::Mas`] — never
/// actually `exec`'d (the App Sandbox gives this build no exec rights
/// over `/usr/bin/open` at all), only matched on by
/// `platform::launcher::spawn_detached`, which routes them to
/// `super::workspace` instead of `std::process::Command`. `args` holds
/// `[bundle_path, url]` for [`MAS_OPEN_WITH_APP`], `[url]` alone for
/// [`MAS_OPEN_URL`].
pub(crate) const MAS_OPEN_WITH_APP: &str = "linh-mas:open-with-app";
pub(crate) const MAS_OPEN_URL: &str = "linh-mas:open-url";

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
    let InstallKind::Native { exec: bundle_path } = &browser.install else {
        return Err("macOS does not support Flatpak launches".into());
    };
    if matches!(bundle_path.as_str(), "msteams:" | "zoommtg:") {
        let program = if current_dist() == Dist::Mas {
            MAS_OPEN_URL
        } else {
            OPEN
        };
        return Ok(LaunchCommand {
            program: program.to_string(),
            args: vec![url.to_string()],
        });
    }

    if current_dist() == Dist::Mas {
        // Sandbox: no exec rights for `/usr/bin/open`, and
        // `NSWorkspace.open(urls:withApplicationAt:configuration:)`
        // never forwards launch arguments to the opened app regardless
        // (see `workspace.rs`'s doc comment) — `target.profile`/
        // `target.private` are ignored here rather than consulted at
        // all, as defense in depth: `routing::degrade_target` has
        // already cleared both before this is ever called.
        return Ok(LaunchCommand {
            program: MAS_OPEN_WITH_APP.to_string(),
            args: vec![bundle_path.clone(), url.to_string()],
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

    if app_args.is_empty() {
        // No flags to deliver: a plain positional URL reaches a running
        // instance as well as launching a new one.
        return Ok(LaunchCommand {
            program: OPEN.to_string(),
            args: vec!["-a".to_string(), bundle_path.clone(), url.to_string()],
        });
    }

    if browser.family == BrowserFamily::Chromium {
        // Guards against the URL being parsed as further flags —
        // everything from here on is the single navigation target.
        app_args.push("--single-argument".to_string());
    }
    app_args.push(url.to_string());

    let mut args = vec![
        "-n".to_string(),
        "-a".to_string(),
        bundle_path.clone(),
        "--args".to_string(),
    ];
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
    fn no_flag_chrome_uses_open_with_a_positional_url() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, true);
        let cmd =
            build_launch_command(&browser, &target(None, false), "https://example.com/").unwrap();
        assert_eq!(cmd.program, OPEN);
        assert_eq!(
            cmd.args,
            vec![
                "-a".to_string(),
                "/Applications/Google Chrome.app".to_string(),
                "https://example.com/".to_string(),
            ],
            "no --args: a positional URL is delivered by Apple Event even \
             to an already-running Chrome"
        );
    }

    #[test]
    fn chrome_profile_and_incognito_force_a_new_instance_with_the_guard_before_url() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, true);
        let cmd = build_launch_command(
            &browser,
            &target(Some("Work"), true),
            "https://example.com/",
        )
        .unwrap();
        assert_eq!(cmd.program, OPEN);
        assert_eq!(
            cmd.args,
            vec![
                "-n".to_string(),
                "-a".to_string(),
                "/Applications/Google Chrome.app".to_string(),
                "--args".to_string(),
                "--profile-directory=Work".to_string(),
                "--incognito".to_string(),
                "--single-argument".to_string(),
                "https://example.com/".to_string(),
            ],
            "flags present: -n forces a new instance so --args actually \
             reaches Chrome's own argv even when it's already running"
        );
    }

    #[test]
    fn firefox_private_forces_a_new_instance_with_the_single_dash_flag() {
        let browser = browser("Firefox", BrowserFamily::Firefox, true);
        let cmd =
            build_launch_command(&browser, &target(None, true), "https://example.com/").unwrap();
        assert_eq!(cmd.program, OPEN);
        assert_eq!(
            cmd.args,
            vec![
                "-n".to_string(),
                "-a".to_string(),
                "/Applications/Firefox.app".to_string(),
                "--args".to_string(),
                "-private-window".to_string(),
                "https://example.com/".to_string(),
            ],
            "no --single-argument guard for Firefox, which has no such flag"
        );
    }

    #[test]
    fn safari_has_no_private_flag_so_still_uses_open_with_a_positional_url() {
        let browser = browser("Safari", BrowserFamily::Other, false);
        let cmd =
            build_launch_command(&browser, &target(None, true), "https://example.com/").unwrap();
        assert_eq!(cmd.program, OPEN);
        assert_eq!(
            cmd.args,
            vec![
                "-a".to_string(),
                "/Applications/Safari.app".to_string(),
                "https://example.com/".to_string(),
            ],
            "private was requested but Safari has no private flag, so no \
             flags end up needing delivery and the plain positional path \
             is used, same as the no-flag case"
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
        assert_eq!(cmd.program, OPEN);
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
    fn rejects_switch_shaped_url_with_flags_present_too() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, true);
        assert!(build_launch_command(&browser, &target(Some("Work"), false), "--evil").is_err());
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
