use std::env;

use linh_core::Browser;
use winreg::enums::KEY_WRITE;

use super::registry::{current_user, read_string};
use super::shell::open_uri;

const PROG_ID: &str = "LinhURL";
const APP_NAME: &str = "Linh";
const ASSOCIATIONS: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations";

fn user_choice(scheme: &str) -> Option<String> {
    read_string(
        &current_user(),
        &format!(r"{ASSOCIATIONS}\{scheme}\UserChoice"),
        "ProgId",
    )
}

fn is_linh_progid(progid: &str) -> bool {
    progid.eq_ignore_ascii_case(PROG_ID)
}

pub fn current_default_browser_id() -> Option<String> {
    user_choice("http")
}

pub fn is_default_browser() -> bool {
    ["http", "https"]
        .iter()
        .all(|scheme| user_choice(scheme).is_some_and(|id| is_linh_progid(&id)))
}

fn create_value(path: &str, name: &str, value: &str) -> Result<(), String> {
    let root = current_user();
    let (key, _) = root
        .create_subkey_with_flags(path, KEY_WRITE)
        .map_err(|e| e.to_string())?;
    key.set_value(name, &value).map_err(|e| e.to_string())
}

pub fn set_default_browser() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_str().ok_or("Linh executable path is not UTF-8")?;
    let command = format!("\"{}\" \"%1\"", exe.replace('"', ""));
    create_value(r"Software\Classes\LinhURL", "", "Linh URL")?;
    create_value(r"Software\Classes\LinhURL", "URL Protocol", "")?;
    create_value(
        r"Software\Classes\LinhURL\DefaultIcon",
        "",
        &format!("\"{exe}\",0"),
    )?;
    create_value(r"Software\Classes\LinhURL\shell\open\command", "", &command)?;
    create_value(r"Software\Linh\Capabilities", "ApplicationName", APP_NAME)?;
    create_value(
        r"Software\Linh\Capabilities",
        "ApplicationDescription",
        "Linh browser picker",
    )?;
    for scheme in ["http", "https"] {
        create_value(
            r"Software\Linh\Capabilities\URLAssociations",
            scheme,
            PROG_ID,
        )?;
    }
    create_value(
        r"Software\RegisteredApplications",
        APP_NAME,
        r"Software\Linh\Capabilities",
    )?;
    open_uri("ms-settings:defaultapps?registeredAppUser=Linh")
}

pub fn matches_desktop_id(browser: &Browser, id: &str) -> bool {
    let normalized = id.to_ascii_lowercase();
    let browser_id = browser.id.to_ascii_lowercase();
    if normalized == browser_id {
        return true;
    }
    let expected = if browser_id.contains("chrome") {
        "chromehtml"
    } else if browser_id.contains("edge") {
        "microsoftedgehtm"
    } else if browser_id.contains("firefox") {
        "firefoxurl"
    } else if browser_id.contains("brave") {
        "bravehtml"
    } else if browser_id.contains("vivaldi") {
        "vivaldihtm"
    } else if browser_id.contains("opera") {
        "opera."
    } else {
        return false;
    };
    normalized.starts_with(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{BrowserFamily, InstallKind};
    #[test]
    fn progid_mapping_is_exact_for_linh() {
        assert!(is_linh_progid("linhurl"));
        assert!(!is_linh_progid("LinhURL.attack"));
    }
    #[test]
    fn known_browser_progid_mapping() {
        let browser = Browser {
            id: "Google Chrome".into(),
            display_name: "Chrome".into(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: String::new(),
            },
            profiles: vec![],
            supports_private: true,
            icon: None,
            desktop_file: None,
        };
        assert!(matches_desktop_id(&browser, "ChromeHTML"));
        assert!(!matches_desktop_id(&browser, "FirefoxURL-123"));
    }
}
