use crate::browser::{Browser, BrowserFamily, InstallKind};
use crate::config::Target;

/// A ready-to-exec program + argv, built as separate tokens so the URL is
/// never shell-interpolated.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Builds the program + args to launch `browser` at `url`, honoring the
/// target's profile/private-window settings.
pub fn build_launch_command(browser: &Browser, target: &Target, url: &str) -> LaunchCommand {
    let flags = family_flags(browser.family, target);

    match &browser.install {
        InstallKind::Flatpak { app_id } => {
            let mut args = vec!["run".to_string(), app_id.clone()];
            args.extend(flags);
            args.push(url.to_string());
            LaunchCommand {
                program: "flatpak".to_string(),
                args,
            }
        }
        InstallKind::Native { exec } => build_native_command(exec, &flags, url),
    }
}

fn family_flags(family: BrowserFamily, target: &Target) -> Vec<String> {
    let mut flags = Vec::new();
    match family {
        BrowserFamily::Chromium => {
            if let Some(profile) = &target.profile {
                flags.push(format!("--profile-directory={profile}"));
            }
            if target.private {
                flags.push("--incognito".to_string());
            }
        }
        BrowserFamily::Firefox => {
            // Firefox profile selection (`-P <name>`) is out of scope for
            // this MVP; only the private-window flag is supported.
            if target.private {
                flags.push("--private-window".to_string());
            }
        }
        BrowserFamily::Other => {}
    }
    flags
}

/// Builds a [`LaunchCommand`] from a `.desktop` `Exec=` line, following the
/// Desktop Entry Specification's quoting and field-code rules:
/// <https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html>
///
/// - The line is tokenized respecting double-quoted arguments (with
///   backslash-escaping inside quotes) so a quoted program path or
///   argument containing spaces stays a single token.
/// - Fixed arguments (anything that isn't a field code) are preserved.
/// - `%u`/`%U`/`%f`/`%F` expand, in place, to `url` as a single argv
///   element — only the first occurrence; any later occurrence is
///   dropped.
/// - Deprecated/unsupported codes (`%d %D %n %N %v %m %i %c %k`) are
///   dropped.
/// - `%%` becomes a literal `%`.
/// - If no URL field code appears anywhere, `url` is appended as the
///   final argument.
/// - `flags` (profile/private) are inserted immediately before the URL
///   argument, wherever it ends up.
fn build_native_command(exec: &str, flags: &[String], url: &str) -> LaunchCommand {
    let mut tokens = tokenize_exec(exec).into_iter();
    let program = tokens.next().unwrap_or_default();

    let mut args: Vec<String> = Vec::new();
    let mut url_used = false;
    let mut url_arg_index: Option<usize> = None;

    for token in tokens {
        let used_before = url_used;
        let expanded = expand_field_codes(&token, url, &mut url_used);
        if expanded.is_empty() {
            continue;
        }
        if !used_before && url_used {
            url_arg_index = Some(args.len());
        }
        args.push(expanded);
    }

    match url_arg_index {
        Some(idx) => {
            let mut result = args[..idx].to_vec();
            result.extend_from_slice(flags);
            result.extend_from_slice(&args[idx..]);
            LaunchCommand {
                program,
                args: result,
            }
        }
        None => {
            args.extend_from_slice(flags);
            args.push(url.to_string());
            LaunchCommand { program, args }
        }
    }
}

/// Tokenizes an `Exec=` line into argv-style tokens, honoring double-quoted
/// arguments (which may contain spaces) and backslash escapes.
fn tokenize_exec(exec: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = exec.chars();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '"' => {
                in_token = true;
                while let Some(next) = chars.next() {
                    match next {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some(escaped @ ('"' | '\\' | '$' | '`')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => current.push('\\'),
                        },
                        other => current.push(other),
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(next) => current.push(next),
                    None => current.push('\\'),
                }
            }
            other => {
                in_token = true;
                current.push(other);
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

const URL_FIELD_CODES: [char; 4] = ['u', 'U', 'f', 'F'];
const DROPPED_FIELD_CODES: [char; 9] = ['d', 'D', 'n', 'N', 'v', 'm', 'i', 'c', 'k'];

/// Expands `%`-field codes in a single token, tracking (via `url_used`)
/// whether the URL has already been placed so later occurrences are
/// dropped instead of duplicating it.
fn expand_field_codes(token: &str, url: &str, url_used: &mut bool) -> String {
    let mut out = String::new();
    let mut chars = token.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some(code) if URL_FIELD_CODES.contains(&code) => {
                chars.next();
                if !*url_used {
                    out.push_str(url);
                    *url_used = true;
                }
                // else: a later URL field code; drop it.
            }
            Some(code) if DROPPED_FIELD_CODES.contains(&code) => {
                chars.next();
                // Deprecated/unsupported code; drop it.
            }
            _ => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::BrowserFamily;

    fn chromium_native(exec: &str) -> Browser {
        Browser {
            id: "chrome".to_string(),
            display_name: "Google Chrome".to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: exec.to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
        }
    }

    fn firefox_flatpak() -> Browser {
        Browser {
            id: "firefox".to_string(),
            display_name: "Firefox".to_string(),
            family: BrowserFamily::Firefox,
            install: InstallKind::Flatpak {
                app_id: "org.mozilla.firefox".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
        }
    }

    fn plain_target() -> Target {
        Target {
            browser: "ignored".to_string(),
            profile: None,
            private: false,
        }
    }

    #[test]
    fn native_chromium_plain() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/google-chrome-stable %U"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(cmd.program, "/usr/bin/google-chrome-stable");
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn native_chromium_profile_and_private() {
        let target = Target {
            browser: "chrome".to_string(),
            profile: Some("Work".to_string()),
            private: true,
        };
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/google-chrome-stable %U"),
            &target,
            "https://example.com",
        );
        assert_eq!(cmd.program, "/usr/bin/google-chrome-stable");
        assert_eq!(
            cmd.args,
            vec![
                "--profile-directory=Work".to_string(),
                "--incognito".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn flatpak_firefox_private() {
        let target = Target {
            browser: "firefox".to_string(),
            profile: None,
            private: true,
        };
        let cmd = build_launch_command(&firefox_flatpak(), &target, "https://example.com");
        assert_eq!(cmd.program, "flatpak");
        assert_eq!(
            cmd.args,
            vec![
                "run".to_string(),
                "org.mozilla.firefox".to_string(),
                "--private-window".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn flatpak_firefox_profile_is_ignored() {
        // Firefox profile routing is out of scope for this MVP: `profile`
        // must not produce a `-P` flag.
        let target = Target {
            browser: "firefox".to_string(),
            profile: Some("default".to_string()),
            private: false,
        };
        let cmd = build_launch_command(&firefox_flatpak(), &target, "https://example.com");
        assert_eq!(cmd.program, "flatpak");
        assert_eq!(
            cmd.args,
            vec![
                "run".to_string(),
                "org.mozilla.firefox".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn native_firefox_profile_is_ignored() {
        let browser = Browser {
            id: "firefox".to_string(),
            display_name: "Firefox".to_string(),
            family: BrowserFamily::Firefox,
            install: InstallKind::Native {
                exec: "/usr/bin/firefox %u".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
        };
        let target = Target {
            browser: "firefox".to_string(),
            profile: Some("default".to_string()),
            private: true,
        };
        let cmd = build_launch_command(&browser, &target, "https://example.com");
        assert_eq!(cmd.program, "/usr/bin/firefox");
        assert_eq!(
            cmd.args,
            vec![
                "--private-window".to_string(),
                "https://example.com".to_string()
            ]
        );
    }

    #[test]
    fn url_is_never_shell_interpolated_even_with_special_chars() {
        let url = "https://example.com/?q=a;rm -rf ~ && echo pwned";
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/google-chrome-stable %U"),
            &plain_target(),
            url,
        );
        // The URL must appear verbatim as a single argv element, never
        // concatenated into a shell string.
        assert_eq!(cmd.args.last().unwrap(), url);
        assert_eq!(cmd.args.len(), 1);
    }

    #[test]
    fn malicious_url_with_quotes_and_semicolons_stays_one_argv_element() {
        let url = r#"https://evil.example/"; rm -rf ~ #"#;
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/google-chrome-stable %u"),
            &plain_target(),
            url,
        );
        assert_eq!(cmd.args, vec![url.to_string()]);
    }

    #[test]
    fn quoted_program_path_with_spaces() {
        let cmd = build_launch_command(
            &chromium_native("\"/opt/My App/app\" %u"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(cmd.program, "/opt/My App/app");
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn extra_fixed_arguments_are_preserved() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --new-window --flag %u"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(
            cmd.args,
            vec![
                "--new-window".to_string(),
                "--flag".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn field_code_in_middle_expands_in_place_and_flags_go_before_it() {
        let target = Target {
            browser: "chrome".to_string(),
            profile: Some("Work".to_string()),
            private: true,
        };
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --before %u --after"),
            &target,
            "https://example.com",
        );
        assert_eq!(
            cmd.args,
            vec![
                "--before".to_string(),
                "--profile-directory=Work".to_string(),
                "--incognito".to_string(),
                "https://example.com".to_string(),
                "--after".to_string(),
            ]
        );
    }

    #[test]
    fn uppercase_u_field_code_expands_to_url() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %U"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn no_field_code_appends_url_at_the_end() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --flag"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(
            cmd.args,
            vec!["--flag".to_string(), "https://example.com".to_string()]
        );
    }

    #[test]
    fn double_percent_is_a_literal_percent() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --title=100%% %u"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(
            cmd.args,
            vec![
                "--title=100%".to_string(),
                "https://example.com".to_string()
            ]
        );
    }

    #[test]
    fn deprecated_field_codes_are_dropped() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %i %c %k %u"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn second_url_field_code_is_dropped_not_duplicated() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %u %U"),
            &plain_target(),
            "https://example.com",
        );
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn quoted_program_path_with_spaces_via_chromium_exec_field_codes_are_stripped() {
        let browser = Browser {
            id: "other".to_string(),
            display_name: "Other".to_string(),
            family: BrowserFamily::Other,
            install: InstallKind::Native {
                exec: "\"/opt/app/bin/app\" --flag %u".to_string(),
            },
            profiles: Vec::new(),
            supports_private: false,
        };
        let cmd = build_launch_command(&browser, &plain_target(), "https://example.com");
        assert_eq!(cmd.program, "/opt/app/bin/app");
        assert_eq!(
            cmd.args,
            vec!["--flag".to_string(), "https://example.com".to_string()]
        );
    }
}
