use std::fs;
use std::path::{Path, PathBuf};

/// The fields of a `.desktop` file's `[Desktop Entry]` group that Linh
/// cares about.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopEntry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    pub hidden: bool,
    pub flatpak_app_id: Option<String>,
    pub path: PathBuf,
}

/// Parses the `[Desktop Entry]` group of a `.desktop` file. Returns `None`
/// if the file cannot be read or has no `Exec=` key (nothing to launch).
/// Locale-suffixed keys (e.g. `Name[vi]=`) are ignored in favor of the
/// unlocalized key.
pub fn parse_desktop_file(path: &Path, id: &str) -> Option<DesktopEntry> {
    let content = fs::read_to_string(path).ok()?;

    let mut in_desktop_entry = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut icon: Option<String> = None;
    let mut mime_types: Vec<String> = Vec::new();
    let mut hidden = false;
    let mut flatpak_app_id: Option<String> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.contains('[') {
            // Localized variant (e.g. `Name[vi]`); skip in favor of the
            // unlocalized key.
            continue;
        }
        let value = value.trim();
        match key {
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "Icon" => icon = Some(value.to_string()),
            "MimeType" => {
                mime_types = value
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "X-Flatpak" => flatpak_app_id = Some(value.to_string()),
            _ => {}
        }
    }

    let exec = exec?;
    Some(DesktopEntry {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        exec,
        icon,
        mime_types,
        hidden,
        flatpak_app_id,
        path: path.to_path_buf(),
    })
}

/// Best-effort extraction of the Flatpak app id from an `Exec=` line of the
/// shape `flatpak run [--flag ...] <app-id> [args...]`.
pub fn flatpak_app_id_from_exec(exec: &str) -> Option<String> {
    let mut tokens = exec.split_whitespace();
    loop {
        let token = tokens.next()?;
        if token == "flatpak" || token.ends_with("/flatpak") {
            break;
        }
    }
    if tokens.next()? != "run" {
        return None;
    }
    for token in tokens {
        if !token.starts_with('-') {
            return Some(token.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).expect("create desktop file");
        file.write_all(content.as_bytes()).expect("write");
        path
    }

    #[test]
    fn parses_basic_fields() {
        let dir = std::env::temp_dir().join(format!("linh-desktop-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_file(
            &dir,
            "google-chrome.desktop",
            "[Desktop Entry]\nName=Google Chrome\nExec=/usr/bin/google-chrome-stable %U\nIcon=google-chrome\nMimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n",
        );
        let entry = parse_desktop_file(&path, "google-chrome").expect("parse");
        assert_eq!(entry.name, "Google Chrome");
        assert_eq!(entry.exec, "/usr/bin/google-chrome-stable %U");
        assert_eq!(entry.icon, Some("google-chrome".to_string()));
        assert!(entry
            .mime_types
            .contains(&"x-scheme-handler/http".to_string()));
        assert!(!entry.hidden);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hidden_entry_is_flagged() {
        let dir =
            std::env::temp_dir().join(format!("linh-desktop-test-hidden-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_file(
            &dir,
            "foo.desktop",
            "[Desktop Entry]\nName=Foo\nExec=/usr/bin/foo %u\nHidden=true\n",
        );
        let entry = parse_desktop_file(&path, "foo").expect("parse");
        assert!(entry.hidden);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn localized_name_is_ignored() {
        let dir =
            std::env::temp_dir().join(format!("linh-desktop-test-locale-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_file(
            &dir,
            "foo.desktop",
            "[Desktop Entry]\nName=Foo\nName[vi]=Fù\nExec=/usr/bin/foo %u\n",
        );
        let entry = parse_desktop_file(&path, "foo").expect("parse");
        assert_eq!(entry.name, "Foo");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_exec_returns_none() {
        let dir =
            std::env::temp_dir().join(format!("linh-desktop-test-noexec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_file(&dir, "foo.desktop", "[Desktop Entry]\nName=Foo\n");
        assert!(parse_desktop_file(&path, "foo").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flatpak_app_id_via_x_flatpak_key() {
        let dir =
            std::env::temp_dir().join(format!("linh-desktop-test-flatpak-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_file(
            &dir,
            "org.mozilla.firefox.desktop",
            "[Desktop Entry]\nName=Firefox\nExec=/usr/bin/flatpak run org.mozilla.firefox %u\nX-Flatpak=org.mozilla.firefox\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
        );
        let entry = parse_desktop_file(&path, "org.mozilla.firefox").expect("parse");
        assert_eq!(
            entry.flatpak_app_id,
            Some("org.mozilla.firefox".to_string())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flatpak_app_id_parsed_from_exec() {
        let app_id = flatpak_app_id_from_exec(
            "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=firefox org.mozilla.firefox @@u %u @@",
        );
        assert_eq!(app_id, Some("org.mozilla.firefox".to_string()));
    }

    #[test]
    fn flatpak_app_id_from_exec_returns_none_for_native_exec() {
        assert_eq!(
            flatpak_app_id_from_exec("/usr/bin/google-chrome-stable %U"),
            None
        );
    }
}
