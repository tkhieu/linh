use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use linh_core::{Browser, Config, OpenIn, Target};

use super::default_browser;
use super::dirs::Dirs;
use crate::platform::logging::log_line;

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

/// Rewrites dropped desktop ids in memory. The next normal config save
/// persists the result; loading alone never modifies an existing file.
pub fn migrate_browser_aliases(
    config: &mut Config,
    aliases: &HashMap<String, String>,
    dirs: &Dirs,
) -> bool {
    if aliases.is_empty() {
        return false;
    }
    let mut changed = rewrite_target(&mut config.primary, aliases);
    for rule in &mut config.rules {
        changed |= match &mut rule.open_in {
            OpenIn::Target(target) => rewrite_target(target, aliases),
            OpenIn::App(app) => app
                .fallback
                .as_mut()
                .is_some_and(|target| rewrite_target(target, aliases)),
            OpenIn::Prompt => false,
        };
    }
    let mut shown_changed = false;
    for key in &mut config.shown_browsers {
        let browser_end = key.find(['/', '#']).unwrap_or(key.len());
        if let Some(kept) = aliases.get(&key[..browser_end]) {
            *key = format!("{}{}", kept, &key[browser_end..]);
            shown_changed = true;
        }
    }
    if shown_changed {
        let mut seen = HashSet::new();
        config.shown_browsers.retain(|key| seen.insert(key.clone()));
        changed = true;
    }
    if changed {
        for (index, rule) in config.rules.iter().enumerate() {
            if config.rules[..index].contains(rule) {
                log_line(
                    dirs,
                    &format!(
                        "browser alias migration kept duplicate rule '{}'",
                        rule.name
                    ),
                );
            }
        }
    }
    changed
}

fn rewrite_target(target: &mut Target, aliases: &HashMap<String, String>) -> bool {
    if let Some(kept) = aliases.get(&target.browser) {
        target.browser.clone_from(kept);
        true
    } else {
        false
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

    #[test]
    fn migrates_saved_chrome_aliases_without_losing_rules_or_writing_on_load() {
        let root = temp_root("alias-migration");
        let path = root.join("linh.toml");
        let original = r##"
shown_browsers = ["google-chrome", "com.google.Chrome", "com.google.Chrome/Default", "google-chrome/Default", "com.google.Chrome#private", "com.google.Chrome/Default#private"]
[primary]
browser = "com.google.Chrome"
profile = "Default"
private = true
[[rules]]
name = "first"
match = ["first.example"]
[rules.open_in]
browser = "com.google.Chrome"
profile = "Default"
[[rules]]
name = "second"
match = ["second.example"]
[rules.open_in]
browser = "google-chrome"
[[rules]]
name = "app fallback"
match = ["third.example"]
[rules.open_in]
app = "zoom"
[rules.open_in.fallback]
browser = "com.google.Chrome"
"##;
        fs::write(&path, original).unwrap();
        let dirs = Dirs {
            home: root.join("home"),
            config_home: root.join("config"),
            state_home: root.join("state"),
            application_dirs: vec![],
        };
        let (mut config, status) = load_or_init(&path, None, &[]);
        assert_eq!(status, ConfigStatus::Loaded);
        let aliases =
            HashMap::from([("com.google.Chrome".to_string(), "google-chrome".to_string())]);
        assert!(migrate_browser_aliases(&mut config, &aliases, &dirs));
        assert_eq!(config.primary.browser, "google-chrome");
        assert_eq!(config.primary.profile.as_deref(), Some("Default"));
        assert!(config.primary.private);
        assert_eq!(
            config.shown_browsers,
            vec![
                "google-chrome",
                "google-chrome/Default",
                "google-chrome#private",
                "google-chrome/Default#private"
            ]
        );
        assert_eq!(config.rules.len(), 3);
        assert!(
            matches!(&config.rules[0].open_in, OpenIn::Target(target) if target.browser == "google-chrome")
        );
        assert!(
            matches!(&config.rules[1].open_in, OpenIn::Target(target) if target.browser == "google-chrome")
        );
        assert!(
            matches!(&config.rules[2].open_in, OpenIn::App(app) if app.fallback.as_ref().is_some_and(|target| target.browser == "google-chrome"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let migrated = config.clone();
        assert!(!migrate_browser_aliases(&mut config, &aliases, &dirs));
        assert_eq!(config, migrated);
        assert!(!migrate_browser_aliases(
            &mut config,
            &HashMap::new(),
            &dirs
        ));
        assert_eq!(config, migrated);
        save(&path, &config).unwrap();
        assert_eq!(load_or_init(&path, None, &[]).0, migrated);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn migration_keeps_rules_that_become_identical_and_no_aliases_are_noop() {
        let root = temp_root("duplicate-rules");
        let dirs = Dirs {
            home: root.join("home"),
            config_home: root.join("config"),
            state_home: root.join("state"),
            application_dirs: vec![],
        };
        let mut config = Config {
            shown_browsers: vec!["com.google.Chrome".into(), "com.google.Chrome".into()],
            ..Config::default()
        };
        let rule = linh_core::Rule {
            name: "same".into(),
            preset: None,
            match_patterns: vec!["example.com".into()],
            source_apps: None,
            open_in: OpenIn::Target(Target {
                browser: "com.google.Chrome".into(),
                profile: None,
                private: false,
            }),
        };
        let mut kept_rule = rule.clone();
        kept_rule.open_in = OpenIn::Target(Target {
            browser: "google-chrome".into(),
            profile: None,
            private: false,
        });
        config.rules = vec![rule, kept_rule.clone()];
        let original = config.clone();
        assert!(!migrate_browser_aliases(
            &mut config,
            &HashMap::new(),
            &dirs
        ));
        assert_eq!(config, original);
        let aliases =
            HashMap::from([("com.google.Chrome".to_string(), "google-chrome".to_string())]);
        assert!(migrate_browser_aliases(&mut config, &aliases, &dirs));
        assert_eq!(config.rules, vec![kept_rule.clone(), kept_rule]);
        assert_eq!(config.shown_browsers, vec!["google-chrome"]);
        fs::remove_dir_all(&root).ok();
    }

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
