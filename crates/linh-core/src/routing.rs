use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{AppTarget, Config, OpenIn, PromptMode, Target};
use crate::matching::pattern_matches;

/// Platform capabilities that affect routing decisions. On Linux,
/// source-app detection is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlatformCaps {
    pub source_app_rules: bool,
}

/// The outcome of a routing decision for a given URL.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Open(Target),
    /// Resolving this (is the app installed? is the URL convertible for
    /// it? did it actually launch?) is platform-specific, so it's left
    /// to the caller — `linh-core` only decides *that* the matching
    /// rule's destination is an app, not how to open it.
    OpenApp(AppTarget),
    Prompt,
}

/// Decides where a URL should be opened, given the config and the
/// platform's capabilities.
///
/// - The first rule whose patterns match the URL wins. Rules
///   that declare `source_apps` are skipped entirely when the platform
///   does not support source-app detection (silently, no error).
/// - If no rule matches, `Always` prompts and `Never` opens the primary target.
pub fn decide(config: &Config, url: &Url, caps: PlatformCaps) -> Decision {
    let host = url.host_str().unwrap_or("").to_string();
    let path = url.path();

    for rule in &config.rules {
        if rule.source_apps.is_some() && !caps.source_app_rules {
            continue;
        }

        let matched = rule
            .match_patterns
            .iter()
            .any(|pattern| pattern_matches(pattern, &host, path));

        if matched {
            return match &rule.open_in {
                OpenIn::Prompt => Decision::Prompt,
                OpenIn::Target(target) => Decision::Open(target.clone()),
                OpenIn::App(app_target) => Decision::OpenApp(app_target.clone()),
            };
        }
    }

    match config.prompt_mode {
        PromptMode::Always => Decision::Prompt,
        PromptMode::Never => Decision::Open(config.primary.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    fn target(browser: &str) -> Target {
        Target {
            browser: browser.to_string(),
            profile: None,
            private: false,
        }
    }

    fn rule(name: &str, patterns: &[&str], open_in: OpenIn) -> Rule {
        Rule {
            name: name.to_string(),
            preset: None,
            match_patterns: patterns.iter().map(|s| s.to_string()).collect(),
            source_apps: None,
            open_in,
        }
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn prompt_mode_always_uses_matching_rule() {
        let mut config = Config {
            prompt_mode: PromptMode::Always,
            ..Config::default()
        };
        config.rules.push(rule(
            "r1",
            &["example.com"],
            OpenIn::Target(target("chrome")),
        ));

        let decision = decide(
            &config,
            &url("https://example.com"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::Open(target("chrome")));
    }

    #[test]
    fn prompt_mode_always_prompts_when_no_rule_matches() {
        let config = Config {
            prompt_mode: PromptMode::Always,
            ..Config::default()
        };
        let decision = decide(
            &config,
            &url("https://unmatched.example"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::Prompt);
    }

    #[test]
    fn first_matching_rule_wins() {
        let mut config = Config::default();
        config.rules.push(rule(
            "general",
            &["company.com"],
            OpenIn::Target(target("chrome")),
        ));
        config.rules.push(rule(
            "specific",
            &["a.company.com"],
            OpenIn::Target(target("firefox")),
        ));

        let decision = decide(
            &config,
            &url("https://a.company.com"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::Open(target("chrome")));
    }

    #[test]
    fn prompt_mode_never_opens_primary_when_no_rule_matches() {
        let config = Config::default();
        let decision = decide(
            &config,
            &url("https://unmatched.example"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::Open(config.primary));
    }

    #[test]
    fn rule_can_resolve_to_an_app_target() {
        let mut config = Config {
            prompt_mode: PromptMode::Always,
            ..Config::default()
        };
        let app_target = AppTarget {
            app: "teams".to_string(),
            fallback: Some(target("google-chrome")),
        };
        config.rules.push(rule(
            "teams",
            &["teams.microsoft.com/l/meetup-join/*"],
            OpenIn::App(app_target.clone()),
        ));

        let decision = decide(
            &config,
            &url("https://teams.microsoft.com/l/meetup-join/19%3aabc"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::OpenApp(app_target));
    }

    #[test]
    fn rule_can_resolve_to_prompt() {
        let mut config = Config::default();
        config
            .rules
            .push(rule("ask", &["example.com"], OpenIn::Prompt));

        let decision = decide(
            &config,
            &url("https://example.com"),
            PlatformCaps::default(),
        );
        assert_eq!(decision, Decision::Prompt);
    }

    #[test]
    fn source_app_rule_skipped_without_capability() {
        let mut config = Config::default();
        let mut r = rule(
            "slack-rule",
            &["example.com"],
            OpenIn::Target(target("chrome")),
        );
        r.source_apps = Some(vec!["Slack".to_string()]);
        config.rules.push(r);

        // Linux: caps.source_app_rules is false, so this rule is skipped
        // silently even though its URL pattern would otherwise match.
        let decision = decide(
            &config,
            &url("https://example.com"),
            PlatformCaps {
                source_app_rules: false,
            },
        );
        assert_eq!(decision, Decision::Open(config.primary));
    }

    #[test]
    fn source_app_rule_evaluated_when_capability_present() {
        let mut config = Config::default();
        let mut r = rule(
            "slack-rule",
            &["example.com"],
            OpenIn::Target(target("chrome")),
        );
        r.source_apps = Some(vec!["Slack".to_string()]);
        config.rules.push(r);

        let decision = decide(
            &config,
            &url("https://example.com"),
            PlatformCaps {
                source_app_rules: true,
            },
        );
        assert_eq!(decision, Decision::Open(target("chrome")));
    }
}
