//! The four built-in rule presets (Google Meet, Microsoft Teams, Slack,
//! Zoom), exposed to the UI via the `list_presets` Tauri command so it
//! can offer "Add preset" prefilled with the plan's frozen `match`
//! patterns. Patterns here are display/prefill metadata only — once a
//! preset is turned into a `Rule`, routing only ever reads
//! `Rule::match_patterns` and `Rule::open_in`, never this module.

use serde::{Deserialize, Serialize};

/// A built-in preset, with `app_installed` filled in by the caller (it
/// requires platform-specific app detection, which this crate — pure
/// routing logic, no Tauri dependency — does not do).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub patterns: Vec<String>,
    pub app: Option<String>,
    pub app_installed: bool,
}

/// The static definition of a preset, before `app_installed` is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetDef {
    pub id: &'static str,
    pub name: &'static str,
    pub patterns: &'static [&'static str],
    /// The desktop app id ("teams" | "zoom") this preset's rule would
    /// route to via `AppTarget`, if any. `None` for presets that always
    /// open in a browser (Meet, Slack).
    pub app: Option<&'static str>,
}

/// The four built-in presets, in display order. These patterns are the
/// plan's frozen Contract section — changing them changes what every
/// "Add preset" rule matches.
pub const PRESET_DEFS: [PresetDef; 4] = [
    PresetDef {
        id: "meet",
        name: "Google Meet",
        patterns: &["meet.google.com"],
        app: None,
    },
    PresetDef {
        id: "teams",
        name: "Microsoft Teams",
        patterns: &[
            "teams.microsoft.com/l/meetup-join/*",
            "teams.microsoft.com/meet/*",
            "teams.live.com/meet/*",
            "teams.live.com/l/meetup-join/*",
        ],
        app: Some("teams"),
    },
    PresetDef {
        id: "slack",
        name: "Slack",
        patterns: &["*.slack.com/archives/*", "app.slack.com/client/*"],
        app: None,
    },
    PresetDef {
        id: "zoom",
        name: "Zoom",
        patterns: &["zoom.us/j/*", "*.zoom.us/j/*"],
        app: Some("zoom"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::pattern_matches;
    use url::Url;

    fn def(id: &str) -> &'static PresetDef {
        PRESET_DEFS.iter().find(|p| p.id == id).unwrap()
    }

    fn matches_any(def: &PresetDef, url: &str) -> bool {
        let parsed = Url::parse(url).expect("valid test URL");
        let host = parsed.host_str().unwrap_or("");
        let path = parsed.path();
        def.patterns
            .iter()
            .any(|pattern| pattern_matches(pattern, host, path))
    }

    #[test]
    fn exactly_four_presets_with_the_expected_ids() {
        let ids: Vec<&str> = PRESET_DEFS.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["meet", "teams", "slack", "zoom"]);
    }

    #[test]
    fn meet_matches_meeting_link() {
        assert!(matches_any(
            def("meet"),
            "https://meet.google.com/abc-defg-hij"
        ));
    }

    #[test]
    fn meet_app_is_none() {
        assert_eq!(def("meet").app, None);
    }

    #[test]
    fn teams_matches_meetup_join_link() {
        assert!(matches_any(
            def("teams"),
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7B%22Tid%22%3A%22x%22%7D"
        ));
    }

    #[test]
    fn teams_matches_live_meetup_join_link() {
        assert!(matches_any(
            def("teams"),
            "https://teams.live.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0"
        ));
    }

    #[test]
    fn teams_does_not_match_admin_pages() {
        assert!(!matches_any(
            def("teams"),
            "https://teams.microsoft.com/v2/"
        ));
    }

    #[test]
    fn teams_app_id_is_teams() {
        assert_eq!(def("teams").app, Some("teams"));
    }

    #[test]
    fn slack_matches_archive_link() {
        assert!(matches_any(
            def("slack"),
            "https://acme.slack.com/archives/C123/p456"
        ));
    }

    #[test]
    fn slack_matches_client_link() {
        assert!(matches_any(
            def("slack"),
            "https://app.slack.com/client/T123/C456"
        ));
    }

    #[test]
    fn slack_does_not_match_bare_domain() {
        assert!(!matches_any(def("slack"), "https://slack.com/"));
    }

    #[test]
    fn slack_app_is_none() {
        assert_eq!(def("slack").app, None);
    }

    #[test]
    fn zoom_matches_join_link() {
        assert!(matches_any(
            def("zoom"),
            "https://us02web.zoom.us/j/123456789?pwd=abc"
        ));
    }

    #[test]
    fn zoom_matches_bare_domain_join_link() {
        assert!(matches_any(def("zoom"), "https://zoom.us/j/123456789"));
    }

    #[test]
    fn zoom_does_not_match_marketing_page() {
        assert!(!matches_any(def("zoom"), "https://zoom.us/pricing"));
    }

    #[test]
    fn zoom_app_id_is_zoom() {
        assert_eq!(def("zoom").app, Some("zoom"));
    }
}
