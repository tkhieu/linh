use crate::browser::{Browser, BrowserFamily, InstallKind};
use crate::config::Target;

/// A ready-to-exec program + argv, built as separate tokens so the URL is
/// never shell-interpolated.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// The `Exec=` line could not be turned into a safe launch command.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchError {
    /// A `%`-field code was used somewhere other than as an entire,
    /// unquoted argument (e.g. inside a quoted argument, or embedded in a
    /// larger argument such as `--url=%u`), or the code itself is `%f`/`%F`
    /// (local-file codes, unsupported for http/https browsers) or an
    /// unrecognized code.
    #[error("invalid Exec field code: {0}")]
    InvalidExec(String),
}

/// Builds the program + args to launch `browser` at `url`, honoring the
/// target's profile/private-window settings. Returns an error if the
/// browser's `Exec=` line uses a field code unsafely (see [`LaunchError`]),
/// in which case the browser should be treated as not launchable.
pub fn build_launch_command(
    browser: &Browser,
    target: &Target,
    url: &str,
) -> Result<LaunchCommand, LaunchError> {
    let flags = family_flags(browser.family, target);

    match &browser.install {
        InstallKind::Flatpak { app_id } => {
            let mut args = vec!["run".to_string(), app_id.clone()];
            args.extend(flags);
            args.push(url.to_string());
            Ok(LaunchCommand {
                program: "flatpak".to_string(),
                args,
            })
        }
        InstallKind::Native { exec } => build_native_command(exec, &flags, url, browser),
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

/// A token produced by [`tokenize_exec`], along with whether any part of
/// it came from inside a double-quoted segment.
struct ExecToken {
    content: String,
    quoted: bool,
}

/// Builds a [`LaunchCommand`] from a `.desktop` `Exec=` line, following the
/// Desktop Entry Specification's string-escaping, quoting, and field-code
/// rules: <https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html>
///
/// Parsing happens in two layers:
/// 1. The desktop-entry string unescape (`\s \n \t \r \\`) is applied to
///    the raw value.
/// 2. The result is tokenized per the `Exec` key's quoting rules
///    (double-quoted arguments, with `\" \` \$ \\`` unescaped inside
///    quotes).
///
/// A field code (`%` followed by a letter, other than the `%%` literal
/// escape) is only valid when it is an entire, unquoted argument on its
/// own; one appearing inside a quoted argument or embedded in a larger
/// argument is rejected, and the URL is never spliced into another
/// argument. `%u`/`%U` expand to `url` (first occurrence only; later ones
/// are dropped); `%f`/`%F` and any unrecognized code are rejected; `%i`
/// expands to `--icon <icon>` when the browser has one (else dropped);
/// `%c` expands to the browser's display name; `%k` expands to the
/// browser's `.desktop` file path when known (else dropped); the
/// deprecated `%d %D %n %N %v %m` are dropped. If no URL code is present,
/// `url` is appended as the final argument. `flags` (profile/private) are
/// inserted immediately before wherever the URL argument ends up.
fn build_native_command(
    exec: &str,
    flags: &[String],
    url: &str,
    browser: &Browser,
) -> Result<LaunchCommand, LaunchError> {
    if exec.trim().is_empty() {
        return Err(LaunchError::InvalidExec("Exec is empty".to_string()));
    }
    let unescaped = unescape_desktop_string(exec);
    let mut tokens = tokenize_exec(&unescaped)?.into_iter();

    let program_token = tokens.next().unwrap_or(ExecToken {
        content: String::new(),
        quoted: false,
    });
    let program = literal_token(&program_token)?;

    let mut args: Vec<String> = Vec::new();
    let mut url_used = false;
    let mut url_arg_index: Option<usize> = None;

    for token in tokens {
        if let Some(code) = bare_field_code(&token) {
            let produced = expand_field_code(code, browser, url, &mut url_used)?;
            if code == 'u' || code == 'U' {
                if let Some(arg) = produced.first() {
                    debug_assert_eq!(produced.len(), 1);
                    url_arg_index = Some(args.len());
                    args.push(arg.clone());
                }
                // else: a later URL code; nothing produced, nothing to push.
            } else {
                args.extend(produced);
            }
        } else {
            args.push(literal_token(&token)?);
        }
    }

    match url_arg_index {
        Some(idx) => {
            let mut result = args[..idx].to_vec();
            result.extend_from_slice(flags);
            result.extend_from_slice(&args[idx..]);
            Ok(LaunchCommand {
                program,
                args: result,
            })
        }
        None => {
            args.extend_from_slice(flags);
            args.push(url.to_string());
            Ok(LaunchCommand { program, args })
        }
    }
}

/// If `token` is entirely unquoted and its full content is exactly `%`
/// followed by one other character (i.e. a standalone field code, not the
/// `%%` literal-percent escape), returns that code character.
fn bare_field_code(token: &ExecToken) -> Option<char> {
    if token.quoted {
        return None;
    }
    let mut chars = token.content.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if first == '%' && second != '%' {
        Some(second)
    } else {
        None
    }
}

/// Expands a single standalone field code into the argv elements it
/// contributes (zero, one, or two).
fn expand_field_code(
    code: char,
    browser: &Browser,
    url: &str,
    url_used: &mut bool,
) -> Result<Vec<String>, LaunchError> {
    match code {
        'u' | 'U' => {
            if *url_used {
                Ok(Vec::new())
            } else {
                *url_used = true;
                Ok(vec![url.to_string()])
            }
        }
        'f' | 'F' => Err(LaunchError::InvalidExec(format!(
            "%{code} requires a local file path and is not supported"
        ))),
        'i' => Ok(match &browser.icon {
            Some(icon) => vec!["--icon".to_string(), icon.clone()],
            None => Vec::new(),
        }),
        'c' => Ok(vec![browser.display_name.clone()]),
        'k' => Ok(match &browser.desktop_file {
            Some(path) => vec![path.clone()],
            None => Vec::new(),
        }),
        'd' | 'D' | 'n' | 'N' | 'v' | 'm' => Ok(Vec::new()),
        other => Err(LaunchError::InvalidExec(format!(
            "unknown field code %{other}"
        ))),
    }
}

/// Resolves a non-field-code token to its literal argument text: `%%`
/// becomes a literal `%`, and any other bare `%` (embedded in a larger or
/// quoted argument) is rejected.
fn literal_token(token: &ExecToken) -> Result<String, LaunchError> {
    if contains_bare_percent(&token.content) {
        return Err(LaunchError::InvalidExec(format!(
            "field code used inside a quoted or non-standalone argument: {:?}",
            token.content
        )));
    }
    Ok(unescape_percent_literal(&token.content))
}

/// True if `s` contains a `%` that is not part of a `%%` pair.
fn contains_bare_percent(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if chars.peek() == Some(&'%') {
                chars.next();
            } else {
                return true;
            }
        }
    }
    false
}

/// Replaces `%%` with a literal `%`. Only call once [`contains_bare_percent`]
/// has confirmed there is no other, invalid use of `%`.
fn unescape_percent_literal(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
        } else {
            out.push(c);
        }
    }
    out
}

/// Layer 1: the general desktop-entry string unescape, applied to the raw
/// `Exec=` value before any `Exec`-specific quoting is parsed. Only
/// `\s \n \t \r \\` are recognized here; any other backslash sequence is
/// left untouched for the `Exec` quoting layer to interpret.
fn unescape_desktop_string(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('s') => {
                chars.next();
                out.push(' ');
            }
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('r') => {
                chars.next();
                out.push('\r');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// Layer 2: tokenizes an (already desktop-string-unescaped) `Exec=` line
/// into argv-style tokens, honoring double-quoted arguments (which may
/// contain spaces) and backslash escapes inside them. Returns
/// [`LaunchError::InvalidExec`] if a double-quoted argument is never
/// closed.
fn tokenize_exec(exec: &str) -> Result<Vec<ExecToken>, LaunchError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quoted = false;
    let mut chars = exec.chars();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_token {
                    tokens.push(ExecToken {
                        content: std::mem::take(&mut current),
                        quoted,
                    });
                    in_token = false;
                    quoted = false;
                }
            }
            '"' => {
                in_token = true;
                quoted = true;
                let mut closed = false;
                while let Some(next) = chars.next() {
                    match next {
                        '"' => {
                            closed = true;
                            break;
                        }
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
                if !closed {
                    return Err(LaunchError::InvalidExec(
                        "unmatched quote in Exec".to_string(),
                    ));
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
        tokens.push(ExecToken {
            content: current,
            quoted,
        });
    }
    Ok(tokens)
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
            icon: None,
            desktop_file: None,
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
            icon: None,
            desktop_file: None,
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
        )
        .unwrap();
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
        )
        .unwrap();
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
        let cmd = build_launch_command(&firefox_flatpak(), &target, "https://example.com").unwrap();
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
        let cmd = build_launch_command(&firefox_flatpak(), &target, "https://example.com").unwrap();
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
            icon: None,
            desktop_file: None,
        };
        let target = Target {
            browser: "firefox".to_string(),
            profile: Some("default".to_string()),
            private: true,
        };
        let cmd = build_launch_command(&browser, &target, "https://example.com").unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
        assert_eq!(cmd.args, vec![url.to_string()]);
    }

    #[test]
    fn quoted_program_path_with_spaces() {
        let cmd = build_launch_command(
            &chromium_native("\"/opt/My App/app\" %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(cmd.program, "/opt/My App/app");
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn extra_fixed_arguments_are_preserved() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --new-window --flag %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn no_field_code_appends_url_at_the_end() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app --flag"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
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
        )
        .unwrap();
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
            &chromium_native("/usr/bin/app %i %c %k %d %D %n %N %v %m %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        // %i (no icon), %k (no desktop file) drop entirely; %c expands to
        // the display name; the rest are deprecated and dropped.
        assert_eq!(
            cmd.args,
            vec![
                "Google Chrome".to_string(),
                "https://example.com".to_string()
            ]
        );
    }

    #[test]
    fn second_url_field_code_is_dropped_not_duplicated() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %u %U"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn quoted_exec_program_path_field_codes_are_still_expanded() {
        let browser = Browser {
            id: "other".to_string(),
            display_name: "Other".to_string(),
            family: BrowserFamily::Other,
            install: InstallKind::Native {
                exec: "\"/opt/app/bin/app\" --flag %u".to_string(),
            },
            profiles: Vec::new(),
            supports_private: false,
            icon: None,
            desktop_file: None,
        };
        let cmd = build_launch_command(&browser, &plain_target(), "https://example.com").unwrap();
        assert_eq!(cmd.program, "/opt/app/bin/app");
        assert_eq!(
            cmd.args,
            vec!["--flag".to_string(), "https://example.com".to_string()]
        );
    }

    #[test]
    fn quoted_field_code_is_rejected() {
        let err = build_launch_command(
            &chromium_native(r#"/usr/bin/app "%u""#),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn field_code_embedded_in_quoted_argument_is_rejected() {
        let err = build_launch_command(
            &chromium_native(r#"/bin/sh -c "browser %u""#),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn field_code_embedded_in_unquoted_argument_is_rejected() {
        let err = build_launch_command(
            &chromium_native("/usr/bin/app --url=%u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn lowercase_f_field_code_is_rejected() {
        let err = build_launch_command(
            &chromium_native("/usr/bin/app %f"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn uppercase_f_field_code_is_rejected() {
        let err = build_launch_command(
            &chromium_native("/usr/bin/app %F"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn empty_exec_is_rejected() {
        let err =
            build_launch_command(&chromium_native(""), &plain_target(), "https://example.com")
                .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn whitespace_only_exec_is_rejected() {
        let err = build_launch_command(
            &chromium_native("   "),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn unmatched_quote_is_rejected() {
        let err = build_launch_command(
            &chromium_native(r#"/usr/bin/app "unterminated %u"#),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn unknown_field_code_is_rejected() {
        let err = build_launch_command(
            &chromium_native("/usr/bin/app %z %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::InvalidExec(_)));
    }

    #[test]
    fn icon_field_code_expands_when_browser_has_icon() {
        let mut browser = chromium_native("/usr/bin/app %i %u");
        browser.icon = Some("google-chrome".to_string());
        let cmd = build_launch_command(&browser, &plain_target(), "https://example.com").unwrap();
        assert_eq!(
            cmd.args,
            vec![
                "--icon".to_string(),
                "google-chrome".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn icon_field_code_drops_when_browser_has_no_icon() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %i %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn desktop_file_field_code_expands_when_known() {
        let mut browser = chromium_native("/usr/bin/app %k %u");
        browser.desktop_file = Some("/usr/share/applications/chrome.desktop".to_string());
        let cmd = build_launch_command(&browser, &plain_target(), "https://example.com").unwrap();
        assert_eq!(
            cmd.args,
            vec![
                "/usr/share/applications/chrome.desktop".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn desktop_file_field_code_drops_when_unknown() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %k %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(cmd.args, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn name_field_code_expands_to_display_name() {
        let cmd = build_launch_command(
            &chromium_native("/usr/bin/app %c %u"),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(
            cmd.args,
            vec![
                "Google Chrome".to_string(),
                "https://example.com".to_string()
            ]
        );
    }

    #[test]
    fn two_layer_unescape_then_quote_parsing() {
        // Raw Exec value contains a quoted argument encoding a literal
        // backslash via the desktop-entry string escape `\\`, which must
        // be resolved *before* the Exec quoting layer interprets the
        // resulting `\$` as an escaped dollar sign.
        let exec = r#"/usr/bin/app "a\\$b" %u"#;
        let cmd = build_launch_command(
            &chromium_native(exec),
            &plain_target(),
            "https://example.com",
        )
        .unwrap();
        assert_eq!(
            cmd.args,
            vec!["a$b".to_string(), "https://example.com".to_string()]
        );
    }
}
