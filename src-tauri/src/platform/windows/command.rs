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
        BrowserFamily::Firefox => Some("--private-window"),
        BrowserFamily::Other => None,
    }
}

pub fn build_launch_command(
    browser: &Browser,
    target: &Target,
    url: &str,
) -> Result<LaunchCommand, String> {
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
        if matches!(arg.as_str(), "%1" | "%L" | "%l" | "%u" | "%U") {
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
    if target.private {
        if let Some(flag) = private_flag(browser) {
            args.push(flag.to_string());
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
            split_command_line(r#""C:\Program Files\app.exe" "a\\\"b""#),
            Some(vec![r"C:\Program Files\app.exe".into(), "a\"b".into()])
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
            Some("--private-window")
        );
    }

    #[test]
    fn builds_argv_without_shell_or_literal_placeholder() {
        let browser = Browser {
            id: "Google Chrome".into(),
            display_name: "Chrome".into(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: r#""C:\Program Files\Google\chrome.exe" --single-argument "%1""#.into(),
            },
            profiles: vec![],
            supports_private: true,
            icon: None,
            desktop_file: None,
        };
        let target = Target {
            browser: browser.id.clone(),
            profile: Some("Profile 1".into()),
            private: true,
        };
        let url = "https://example.com/a?x=1&y=2";
        let cmd = build_launch_command(&browser, &target, url).unwrap();
        assert_eq!(cmd.program, r"C:\Program Files\Google\chrome.exe");
        assert_eq!(
            cmd.args,
            [
                "--single-argument",
                "--profile-directory=Profile 1",
                "--incognito",
                url
            ]
        );
    }
}
