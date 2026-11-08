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
        InstallKind::Native { exec } => {
            let program = exec_program(exec);
            let mut args = flags;
            args.push(url.to_string());
            LaunchCommand { program, args }
        }
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
            if let Some(profile) = &target.profile {
                flags.push("-P".to_string());
                flags.push(profile.clone());
            }
            if target.private {
                flags.push("--private-window".to_string());
            }
        }
        BrowserFamily::Other => {}
    }
    flags
}

/// Extracts the program token from a `.desktop` `Exec=` line, dropping
/// field codes (`%u`, `%f`, ...) and any extra arguments.
fn exec_program(exec: &str) -> String {
    exec.split_whitespace()
        .next()
        .unwrap_or(exec)
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::BrowserFamily;

    fn chromium_native() -> Browser {
        Browser {
            id: "chrome".to_string(),
            display_name: "Google Chrome".to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: "/usr/bin/google-chrome-stable %U".to_string(),
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
        let cmd = build_launch_command(&chromium_native(), &plain_target(), "https://example.com");
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
        let cmd = build_launch_command(&chromium_native(), &target, "https://example.com");
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
    fn flatpak_firefox_profile() {
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
                "-P".to_string(),
                "default".to_string(),
                "https://example.com".to_string(),
            ]
        );
    }

    #[test]
    fn url_is_never_shell_interpolated_even_with_special_chars() {
        let url = "https://example.com/?q=a;rm -rf ~ && echo pwned";
        let cmd = build_launch_command(&chromium_native(), &plain_target(), url);
        // The URL must appear verbatim as a single argv element, never
        // concatenated into a shell string.
        assert_eq!(cmd.args.last().unwrap(), url);
        assert_eq!(cmd.args.len(), 1);
    }

    #[test]
    fn exec_field_codes_are_stripped() {
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
    }
}
