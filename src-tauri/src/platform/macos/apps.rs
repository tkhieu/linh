//! Detection of the non-browser desktop apps an `AppTarget` rule can
//! launch (Teams, Zoom), via LaunchServices' registered handler for each
//! app's custom URL scheme.

use linh_core::{Browser, BrowserFamily, InstallKind};
use url::Url;

use super::dirs::Dirs;
use super::launch_services;

fn scheme(app_id: &str) -> Option<&'static str> {
    match app_id {
        "teams" => Some("msteams"),
        "zoom" => Some("zoommtg"),
        _ => None,
    }
}

/// The name shown to the user for `app_id` (e.g. in a "could not confirm
/// opening in ..." notification) — never the bare internal id.
fn friendly_name(app_id: &str) -> &str {
    match app_id {
        "teams" => "Microsoft Teams",
        "zoom" => "Zoom",
        other => other,
    }
}

/// Microsoft documents `msteams://` for Teams deep links under `/l/`.
/// Other Teams web URLs stay on the browser fallback path — the official
/// Teams client on macOS, like on Windows, is a handler for `msteams:`,
/// not for raw `https` URLs.
pub fn teams_app_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str()? != "teams.microsoft.com"
        || !parsed.path().starts_with("/l/")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return None;
    }
    Some(format!(
        "msteams://{}",
        parsed.as_str().strip_prefix("https://")?
    ))
}

/// Finds the installed app (if any) registered as `app_id`'s handler
/// scheme. Returned as a `Browser` purely so `launcher::build_launch_command`
/// (which takes one) can be reused to build its launch argv; its bundle
/// path isn't resolved since `open <scheme>:<...>` dispatches to
/// whichever app LaunchServices has registered without needing it.
pub fn find_app(_dirs: &Dirs, app_id: &str) -> Option<Browser> {
    let scheme = scheme(app_id)?;
    if launch_services::handlers_for_scheme(scheme).is_empty() {
        return None;
    }
    Some(Browser {
        id: app_id.to_string(),
        display_name: friendly_name(app_id).to_string(),
        family: BrowserFamily::Other,
        install: InstallKind::Native {
            exec: format!("{scheme}:"),
        },
        profiles: Vec::new(),
        supports_private: false,
        icon: None,
        desktop_file: None,
    })
}

pub fn is_app_installed(dirs: &Dirs, app_id: &str) -> bool {
    find_app(dirs, app_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_protocols() {
        assert_eq!(scheme("teams"), Some("msteams"));
        assert_eq!(scheme("zoom"), Some("zoommtg"));
        assert_eq!(scheme("slack"), None);
    }

    #[test]
    fn friendly_names_are_shown_not_internal_ids() {
        assert_eq!(friendly_name("teams"), "Microsoft Teams");
        assert_eq!(friendly_name("zoom"), "Zoom");
        assert_eq!(friendly_name("slack"), "slack");
    }

    #[test]
    fn converts_only_teams_deep_links() {
        assert_eq!(
            teams_app_url("https://teams.microsoft.com/l/meetup-join/abc?x=1"),
            Some("msteams://teams.microsoft.com/l/meetup-join/abc?x=1".into())
        );
        assert_eq!(teams_app_url("https://teams.microsoft.com/meet/abc"), None);
        assert_eq!(
            teams_app_url("https://evil.example/l/meetup-join/abc"),
            None
        );
    }
}
