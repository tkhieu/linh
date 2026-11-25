//! Windows registry commands use CRT quoting and `%1`, not Desktop Entry `Exec` syntax.
use linh_core::{Browser, BrowserFamily, InstallKind, LaunchCommand, Target};

use super::registry;

/// Split a Windows command line using the standard backslash-before-quote rule.
fn split_command_line(raw: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut chars = raw.chars().peekable();
    loop {
        while matches!(chars.peek(), Some(' ' | '\t')) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while let Some(&ch) = chars.peek() {
            if !quoted && (ch == ' ' || ch == '\t') {
                break;
            }
            if ch == '\\' {
                let mut count = 0;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    arg.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 0 {
                        quoted = !quoted;
                    } else {
                        arg.push('"');
                    }
                    chars.next();
                } else {
                    arg.extend(std::iter::repeat_n('\\', count));
                }
            } else if ch == '"' {
                quoted = !quoted;
                chars.next();
            } else {
                arg.push(ch);
                chars.next();
            }
        }
        if quoted {
            return None;
        }
        args.push(arg);
    }
    Some(args)
}

pub fn parse_registry_command(raw: &str) -> Option<(String, Vec<String>)> {
    let expanded = registry::expand_env(raw);
    let mut parts = split_command_line(&expanded)?.into_iter();
    let program = parts.next()?;
    let filename = std::path::Path::new(&program)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    if !filename.ends_with(".exe")
        || matches!(
            filename.as_str(),
            "cmd.exe" | "powershell.exe" | "pwsh.exe" | "wscript.exe" | "cscript.exe"
        )
    {
        return None;
    }
    let args: Vec<_> = parts.collect();
    if args
        .iter()
        .any(|arg| arg.contains('%') && !matches!(arg.as_str(), "%1" | "%L" | "%l" | "%u" | "%U"))
    {
        return None;
    }
    Some((program, args))
}

fn private_flag(browser: &Browser) -> Option<&'static str> {
    if !browser.supports_private {
        return None;
    }
    if browser.id.to_ascii_lowercase().contains("opera") {
        return Some("--private");
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
    debug_assert!(!url.starts_with('-'));
    let InstallKind::Native { exec } = &browser.install else {
        return Err("Windows does not support Flatpak launches".into());
    };
    if matches!(exec.as_str(), "msteams:" | "zoommtg:") {
        return Ok(LaunchCommand {
            program: exec.clone(),
            args: vec![url.to_string()],
        });
    }
    let (program, source_args) =
        parse_registry_command(exec).ok_or("invalid Windows registry command")?;
    let mut args = Vec::new();
    for arg in source_args {
        if matches!(arg.as_str(), "%1" | "%L" | "%l" | "%u" | "%U")
            || arg.eq_ignore_ascii_case("--single-argument")
            || (browser.family == BrowserFamily::Firefox
                && ["-osint", "-url", "-private-window"]
                    .iter()
                    .any(|flag| arg.eq_ignore_ascii_case(flag)))
        {
            continue;
        }
        args.push(arg);
    }
    if let Some(profile) = &target.profile {
        if browser.family == BrowserFamily::Chromium
            && !browser.id.to_ascii_lowercase().contains("opera")
        {
            args.push(format!("--profile-directory={profile}"));
        }
    }
    if browser.family == BrowserFamily::Firefox {
        // Firefox's -osint sanitizer accepts only its registered -url or
        // -private-window form, with the URL as the final argument.
        // https://github.com/mozilla-firefox/firefox/blob/main/browser/docs/CommandLineParameters.md
        args.push("-osint".to_string());
        args.push(
            if target.private && browser.supports_private {
                "-private-window"
            } else {
                "-url"
            }
            .to_string(),
        );
    } else {
        if target.private {
            if let Some(flag) = private_flag(browser) {
                args.push(flag.to_string());
            }
        }
        if browser.family == BrowserFamily::Chromium {
            args.push("--single-argument".to_string());
        }
    }
    args.push(url.to_string());
    Ok(LaunchCommand { program, args })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_registry_command_keeps_path_and_fixed_args() {
        assert_eq!(
            parse_registry_command(
                r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#
            ),
            Some((
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                vec!["--single-argument".into(), "%1".into()]
            ))
        );
    }
    #[test]
    fn rejects_shell_and_bad_placeholder() {
        assert!(parse_registry_command(r#""C:\Windows\System32\cmd.exe" /c %1"#).is_none());
        assert!(parse_registry_command(r#""C:\Browser\chrome.exe" --url=%1"#).is_none());
    }
    #[test]
    fn handles_windows_backslash_quote() {
        assert_eq!(
            split_command_line(r#""C:\Program Files\app.exe" "a\"b""#),
            Some(vec![r"C:\Program Files\app.exe".into(), "a\"b".into()])
        );
        assert_eq!(
            split_command_line(r#""C:\Program Files\app.exe" "a\\\"b""#),
            Some(vec![r"C:\Program Files\app.exe".into(), "a\\\"b".into()])
        );
    }
    #[test]
    fn private_flags_cover_known_families() {
        let mk = |id: &str, family| Browser {
            id: id.into(),
            display_name: id.into(),
            family,
            install: InstallKind::Native {
                exec: r"C:\browser.exe".into(),
            },
            profiles: vec![],
            supports_private: true,
            icon: None,
            desktop_file: None,
        };
        assert_eq!(
            private_flag(&mk("Chrome", BrowserFamily::Chromium)),
            Some("--incognito")
        );
        assert_eq!(
            private_flag(&mk("Opera", BrowserFamily::Chromium)),
            Some("--private")
        );
        assert_eq!(
            private_flag(&mk("Firefox", BrowserFamily::Firefox)),
            Some("-private-window")
        );
    }

    fn browser(id: &str, family: BrowserFamily, exec: &str) -> Browser {
        Browser {
            id: id.into(),
            display_name: id.into(),
            family,
            install: InstallKind::Native { exec: exec.into() },
            profiles: vec![],
            supports_private: true,
            icon: None,
            desktop_file: None,
        }
    }

    #[test]
    fn chrome_progid_places_guard_after_profile_and_incognito() {
        let browser = browser(
            "Google Chrome",
            BrowserFamily::Chromium,
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument "%1""#,
        );
        let target = Target {
            browser: browser.id.clone(),
            profile: Some("Profile 1".into()),
            private: true,
        };
        let url = "https://example.com/a?x=1&y=2";
        let cmd = build_launch_command(&browser, &target, url).unwrap();
        assert_eq!(
            cmd.program,
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        );
        assert_eq!(
            cmd.args,
            [
                "--profile-directory=Profile 1",
                "--incognito",
                "--single-argument",
                url
            ]
        );
    }

    #[test]
    fn edge_progid_keeps_fixed_args_before_guard() {
        let browser = browser(
            "Microsoft Edge",
            BrowserFamily::Chromium,
            r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --profile-directory=Default --single-argument "%1""#,
        );
        let target = Target {
            browser: browser.id.clone(),
            profile: None,
            private: true,
        };
        let cmd = build_launch_command(&browser, &target, "https://example.com/").unwrap();
        assert_eq!(
            cmd.program,
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
        );
        assert_eq!(
            cmd.args,
            [
                "--profile-directory=Default",
                "--incognito",
                "--single-argument",
                "https://example.com/"
            ]
        );
    }

    #[test]
    fn firefox_progid_uses_mozilla_osint_forms() {
        let browser = browser(
            "Mozilla Firefox",
            BrowserFamily::Firefox,
            r#""C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url "%1""#,
        );
        let mut target = Target {
            browser: browser.id.clone(),
            profile: None,
            private: false,
        };
        let normal = build_launch_command(&browser, &target, "https://example.com/").unwrap();
        assert_eq!(
            normal.program,
            r"C:\Program Files\Mozilla Firefox\firefox.exe"
        );
        assert_eq!(normal.args, ["-osint", "-url", "https://example.com/"]);
        target.private = true;
        let private = build_launch_command(&browser, &target, "https://example.com/").unwrap();
        assert_eq!(
            private.args,
            ["-osint", "-private-window", "https://example.com/"]
        );
    }

    #[test]
    fn start_menu_command_without_placeholder_still_guards_url() {
        let browser = browser(
            "Google Chrome",
            BrowserFamily::Chromium,
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe""#,
        );
        let target = Target {
            browser: browser.id.clone(),
            profile: Some("Work".into()),
            private: true,
        };
        let cmd = build_launch_command(&browser, &target, "https://example.com/").unwrap();
        assert_eq!(
            cmd.program,
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        );
        assert_eq!(
            cmd.args,
            [
                "--profile-directory=Work",
                "--incognito",
                "--single-argument",
                "https://example.com/"
            ]
        );
    }

    #[test]
    fn rejects_switch_shaped_url() {
        let browser = browser("Google Chrome", BrowserFamily::Chromium, r"C:\chrome.exe");
        let target = Target {
            browser: browser.id.clone(),
            profile: None,
            private: false,
        };
        assert!(build_launch_command(&browser, &target, "--evil").is_err());
    }
}
