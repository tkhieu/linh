use std::env;

use linh_core::Browser;
use winreg::enums::KEY_WRITE;

use crate::platform::logging::log_line;

use super::command::parse_registry_command;
use super::dirs::Dirs;
use super::registry::{current_user, read_string};
use super::shell::open_uri;

const PROG_ID: &str = "LinhURL";
const APP_NAME: &str = "Linh";
const ASSOCIATIONS: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations";
const OPEN_COMMAND: &str = r"Software\Classes\LinhURL\shell\open\command";

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

fn command_for_exe(exe: &str) -> String {
    format!("\"{}\" \"%1\"", exe.replace('"', ""))
}

fn points_to_exe(command: &str, exe: &str) -> bool {
    let Some((registered, _)) = parse_registry_command(command) else {
        return false;
    };
    let normalize = |path: &str| {
        path.trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_ascii_lowercase()
    };
    normalize(&registered) == normalize(exe)
}

/// Repair a per-user registration left pointing at an older installation.
/// No registration is created at startup if the user has never opted in.
pub fn refresh_registration_if_stale(dirs: &Dirs) {
    let Some(existing) = read_string(&current_user(), OPEN_COMMAND, "") else {
        return;
    };
    let exe = match env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            log_line(
                dirs,
                &format!("could not check Linh registration path: {err}"),
            );
            return;
        }
    };
    let Some(exe) = exe.to_str() else {
        log_line(
            dirs,
            "could not check Linh registration path: non-UTF-8 executable path",
        );
        return;
    };
    if points_to_exe(&existing, exe) {
        return;
    }
    match create_value(OPEN_COMMAND, "", &command_for_exe(exe)) {
        Ok(()) => log_line(
            dirs,
            "updated stale Linh URL registration to the current executable",
        ),
        Err(err) => log_line(
            dirs,
            &format!("failed to update stale Linh URL registration: {err}"),
        ),
    }
}

pub fn set_default_browser() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_str().ok_or("Linh executable path is not UTF-8")?;
    let command = command_for_exe(exe);
    create_value(r"Software\Classes\LinhURL", "", "Linh URL")?;
    create_value(r"Software\Classes\LinhURL", "URL Protocol", "")?;
    create_value(
        r"Software\Classes\LinhURL\DefaultIcon",
        "",
        &format!("\"{exe}\",0"),
    )?;
    create_value(OPEN_COMMAND, "", &command)?;
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

    #[test]
    fn stale_registration_path_comparison() {
        assert!(points_to_exe(
            r#""C:\Program Files\Linh\linh.exe" "%1""#,
            r"c:\program files\linh\linh.exe"
        ));
        assert!(!points_to_exe(
            r#""C:\Old\linh.exe" "%1""#,
            r"C:\New\linh.exe"
        ));
        assert!(!points_to_exe("unparseable", r"C:\New\linh.exe"));
    }
}
