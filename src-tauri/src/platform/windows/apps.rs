//! App targets are launched through their registered protocol via ShellExecuteW.
use super::dirs::Dirs;
use super::registry::{classes_root, current_user, read_string};
use linh_core::{Browser, BrowserFamily, InstallKind};
use url::Url;

/// Microsoft documents msteams:// for Teams deep links under /l/.
/// Other Teams web URLs stay on the browser fallback path.
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

fn scheme(app_id: &str) -> Option<&'static str> {
    match app_id {
        "teams" => Some("msteams"),
        "zoom" => Some("zoommtg"),
        _ => None,
    }
}

pub fn find_app(_dirs: &Dirs, app_id: &str) -> Option<Browser> {
    let scheme = scheme(app_id)?;
    let user = current_user();
    let classes = classes_root();
    let user_path = format!(r"Software\Classes\{scheme}");
    let has_handler = [(&user, user_path.as_str()), (&classes, scheme)]
        .iter()
        .any(|(root, path)| {
            read_string(root, &format!(r"{path}\shell\open\command"), "").is_some()
                && read_string(root, path, "URL Protocol").is_some()
        });
    has_handler.then(|| Browser {
        id: app_id.into(),
        display_name: app_id.into(),
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
