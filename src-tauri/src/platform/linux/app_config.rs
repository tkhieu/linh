use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use linh_core::{Browser, Config, Target};

use super::default_browser;

/// Serializes every `save` call process-wide so two threads (e.g. the
/// tray's click handler and a `save_config` command) never write
/// overlapping temp files or race on which one's rename wins.
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// Monotonic counter mixed into each temp file's name so two saves in
/// the same process never collide even if their timing coincided.
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How `load_or_init` resolved the on-disk config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigStatus {
    /// Loaded and parsed successfully.
    Loaded,
    /// No config file existed; a default one was created.
    Created,
    /// A config file existed but failed to parse (or could not be read);
    /// it was left untouched and a fallback config is used instead.
    Broken,
}

/// Loads `linh.toml` from `path`, creating a default one (primary =
/// previously-configured system default browser, if any and not Linh
/// itself, else the first detected browser) if it is missing. If the file
/// exists but fails to parse, it is **not** overwritten; a fallback config
/// (primary = first detected browser) is returned instead so the caller
/// can still route URLs and surface the error to the user.
pub fn load_or_init(
    path: &Path,
    previous_default_desktop_id: Option<&str>,
    detected: &[Browser],
) -> (Config, ConfigStatus) {
    match fs::read_to_string(path) {
        Ok(data) => match linh_core::parse_config(&data) {
            Ok(config) => (config, ConfigStatus::Loaded),
            Err(_) => (fallback_config(detected), ConfigStatus::Broken),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = default_config(previous_default_desktop_id, detected);
            // Best-effort: if we can't create the default file, the
            // config still works for this session, just not persisted.
            let _ = write_atomic(path, &config);
            (config, ConfigStatus::Created)
        }
        Err(_) => (fallback_config(detected), ConfigStatus::Broken),
    }
}

/// Validates `config` and writes it to `path` atomically and serialized
/// against any other concurrent `save` call: a uniquely-named sibling
/// temp file is written, `fsync`ed, then renamed over the target while
/// holding a process-wide lock.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    validate(config)?;
    let _guard = SAVE_LOCK.lock().unwrap();
    write_atomic(path, config).map_err(|e| e.to_string())
}

fn validate(config: &Config) -> Result<(), String> {
    if config.primary.browser.trim().is_empty() {
        return Err("primary.browser must not be empty".to_string());
    }
    for rule in &config.rules {
        if rule.match_patterns.is_empty() {
            return Err(format!("rule '{}' has no match patterns", rule.name));
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, config: &Config) -> io::Result<()> {
    let data = linh_core::serialize_config(config)
        .map_err(|e| io::Error::other(format!("failed to serialize config: {e}")))?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;
    fs::create_dir_all(parent)?;

    let unique = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!("linh.toml.tmp-{}-{unique}", std::process::id()));
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(data.as_bytes())?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, path)
}

fn default_config(previous_default_desktop_id: Option<&str>, detected: &[Browser]) -> Config {
    let primary_id = previous_default_desktop_id
        .and_then(|xdg_id| {
            detected
                .iter()
                .find(|b| default_browser::matches_desktop_id(b, xdg_id))
        })
        .or_else(|| detected.first())
        .map(|b| b.id.clone());
    Config {
        primary: Target {
            browser: primary_id.unwrap_or_else(|| "firefox".to_string()),
            profile: None,
            private: false,
        },
        ..Config::default()
    }
}

fn fallback_config(detected: &[Browser]) -> Config {
    let primary_id = detected.first().map(|b| b.id.clone());
    Config {
        primary: Target {
            browser: primary_id.unwrap_or_else(|| "firefox".to_string()),
            profile: None,
            private: false,
        },
        ..Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{BrowserFamily, InstallKind};
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-appconfig-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn browser(id: &str) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family: BrowserFamily::Chromium,
            install: InstallKind::Native {
                exec: "/usr/bin/x %u".to_string(),
            },
            profiles: Vec::new(),
            supports_private: true,
            icon: None,
            desktop_file: Some(format!("/usr/share/applications/{id}.desktop")),
        }
    }

    #[test]
    fn missing_file_creates_default_with_previous_system_default() {
        let root = temp_root("missing-with-previous");
        let path = root.join("linh.toml");
        let detected = vec![browser("firefox"), browser("google-chrome")];

        let (config, status) = load_or_init(&path, Some("google-chrome.desktop"), &detected);
        assert_eq!(status, ConfigStatus::Created);
        assert_eq!(config.primary.browser, "google-chrome");
        assert!(path.exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_file_falls_back_to_first_detected_when_no_previous_default() {
        let root = temp_root("missing-no-previous");
        let path = root.join("linh.toml");
        let detected = vec![browser("firefox"), browser("google-chrome")];

        let (config, status) = load_or_init(&path, None, &detected);
        assert_eq!(status, ConfigStatus::Created);
        assert_eq!(config.primary.browser, "firefox");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_file_ignores_previous_default_that_is_not_detected() {
        let root = temp_root("missing-unknown-previous");
        let path = root.join("linh.toml");
        let detected = vec![browser("firefox")];

        let (config, status) = load_or_init(&path, Some("some-unknown-browser.desktop"), &detected);
        assert_eq!(status, ConfigStatus::Created);
        assert_eq!(config.primary.browser, "firefox");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn valid_existing_file_is_loaded() {
        let root = temp_root("valid");
        let path = root.join("linh.toml");
        let detected = vec![browser("firefox")];
        let (initial, _) = load_or_init(&path, None, &detected);
        assert_eq!(initial.primary.browser, "firefox");

        let (loaded, status) = load_or_init(&path, None, &detected);
        assert_eq!(status, ConfigStatus::Loaded);
        assert_eq!(loaded, initial);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn broken_file_is_not_overwritten_and_falls_back() {
        let root = temp_root("broken");
        let path = root.join("linh.toml");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "this is not valid toml [[[").unwrap();
        let original = fs::read_to_string(&path).unwrap();

        let detected = vec![browser("firefox"), browser("google-chrome")];
        let (config, status) = load_or_init(&path, None, &detected);
        assert_eq!(status, ConfigStatus::Broken);
        assert_eq!(config.primary.browser, "firefox");
        // The broken file must be left untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn broken_file_with_no_detected_browsers_falls_back_to_firefox() {
        let root = temp_root("broken-no-detected");
        let path = root.join("linh.toml");
        fs::write(&path, "not valid toml [[[").unwrap();

        let (config, status) = load_or_init(&path, None, &[]);
        assert_eq!(status, ConfigStatus::Broken);
        assert_eq!(config.primary.browser, "firefox");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn save_validates_empty_primary_browser() {
        let root = temp_root("validate");
        let path = root.join("linh.toml");
        let mut config = Config::default();
        config.primary.browser = String::new();
        assert!(save(&path, &config).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn save_writes_atomically_and_round_trips() {
        let root = temp_root("save");
        let path = root.join("nested/linh.toml");
        let config = Config::default();
        save(&path, &config).expect("save");
        let (loaded, status) = load_or_init(&path, None, &[]);
        assert_eq!(status, ConfigStatus::Loaded);
        assert_eq!(loaded, config);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn concurrent_saves_leave_a_valid_file() {
        let root = temp_root("concurrent");
        let path = std::sync::Arc::new(root.join("linh.toml"));

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let config = Config {
                        primary: Target {
                            browser: format!("browser-{i}"),
                            profile: None,
                            private: false,
                        },
                        ..Config::default()
                    };
                    save(&path, &config)
                })
            })
            .collect();

        for handle in handles {
            handle
                .join()
                .expect("thread panicked")
                .expect("save failed");
        }

        // Whichever save landed last, the file on disk must parse as a
        // complete, valid config — never truncated or interleaved from
        // two concurrent writers.
        let data = fs::read_to_string(&*path).expect("read final file");
        let config = linh_core::parse_config(&data).expect("final file must parse");
        assert!(config.primary.browser.starts_with("browser-"));

        fs::remove_dir_all(&root).ok();
    }
}
