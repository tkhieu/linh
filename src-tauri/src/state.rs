use std::collections::HashMap;
use std::sync::Mutex;

use linh_core::{Browser, Config, PlatformCaps, Target};
use tauri::menu::CheckMenuItem;
use tauri::Wry;

use crate::platform::current::{app_config, browsers, default_browser, dirs::Dirs, notify};
use crate::prompt_state::PromptState;

/// Shared, mutable application state, managed by Tauri and reachable from
/// every command and from the tray menu's click handler.
pub struct AppState {
    pub dirs: Dirs,
    pub caps: PlatformCaps,
    pub config: Mutex<Config>,
    pub browsers: Mutex<Vec<Browser>>,
    pub prompt: PromptState,
    /// The targets listed in the tray's "Primary Browser" submenu, indexed
    /// by the `primary:<index>` menu item id that was built from them.
    pub primary_targets: Mutex<Vec<Target>>,
    /// The live `CheckMenuItem` handles, same order as `primary_targets`,
    /// used to update which one is checked without replacing the tray's
    /// menu object (unsupported once set, on Linux).
    pub primary_items: Mutex<Vec<CheckMenuItem<Wry>>>,
    /// Memoizes `get_icon`'s result for an icon name/path already looked
    /// up this session, keyed on the exact argument given (so an
    /// absolute path and a theme name never collide).
    pub icon_cache: Mutex<HashMap<String, Option<String>>>,
    /// Serializes every config save (whether a whole-config replacement
    /// from `save_config`, or an in-place `update_config` from the tray)
    /// so two concurrent callers can never leave the in-memory `config`
    /// and the on-disk file pointing at different saves, and so an
    /// `update_config`'s read-modify-write of the current config is
    /// never interleaved with another save.
    save_lock: Mutex<()>,
}

impl AppState {
    /// Detects installed browsers and loads (or recovers) the config,
    /// notifying the user if the config file turned out to be broken.
    pub fn init() -> Self {
        let dirs = Dirs::from_env();
        let detected = browsers::detect_browsers(&dirs);
        let previous_default = default_browser::current_default_browser_id();
        let config_path = dirs.config_path();
        let (config, status) =
            app_config::load_or_init(&config_path, previous_default.as_deref(), &detected);

        if status == app_config::ConfigStatus::Broken {
            notify::notify_error(
                &dirs,
                "Linh: configuration error",
                &format!(
                    "{} could not be parsed and was left untouched. Links will open in {} until it is fixed.",
                    config_path.display(),
                    config.primary.browser
                ),
            );
        }

        AppState {
            dirs,
            caps: PlatformCaps {
                source_app_rules: false,
            },
            config: Mutex::new(config),
            browsers: Mutex::new(detected),
            prompt: PromptState::new(),
            primary_targets: Mutex::new(Vec::new()),
            primary_items: Mutex::new(Vec::new()),
            icon_cache: Mutex::new(HashMap::new()),
            save_lock: Mutex::new(()),
        }
    }

    /// Re-scans installed browsers, updates the cache, and returns the
    /// fresh list.
    pub fn refresh_browsers(&self) -> Vec<Browser> {
        let detected = browsers::detect_browsers(&self.dirs);
        *self.browsers.lock().unwrap() = detected.clone();
        detected
    }

    /// Applies `mutate` to a clone of the *current* in-memory config,
    /// then validates and persists the result and updates the in-memory
    /// copy — reading, mutating, saving, and updating memory all under
    /// `save_lock`, so no other save can read a stale config out from
    /// under `mutate`, and no field `mutate` doesn't touch is ever lost.
    /// Used by the tray, which only ever changes `primary`.
    pub fn update_config(&self, mutate: impl FnOnce(&mut Config)) -> Result<(), String> {
        let _guard = self.save_lock.lock().unwrap();
        let mut config = self.config.lock().unwrap().clone();
        mutate(&mut config);
        app_config::save(&self.dirs.config_path(), &config)?;
        *self.config.lock().unwrap() = config;
        Ok(())
    }

    /// Replaces the whole config (the `save_config` command receives a
    /// complete `Config` from the UI). Implemented via
    /// [`AppState::update_config`] so it shares `save_lock` with it and
    /// the two can never interleave.
    pub fn persist_config(&self, config: Config) -> Result<(), String> {
        self.update_config(move |current| *current = config)
    }

    /// Builds a minimal `AppState` by hand (not `AppState::init()`, which
    /// scans the real filesystem and shells out to `xdg-settings`), for
    /// hermetic unit tests elsewhere in this crate that need a full
    /// `AppState` (e.g. `routing`'s tests) rather than exercising pure
    /// logic directly.
    #[cfg(test)]
    pub(crate) fn for_test(dirs: Dirs, config: Config, browsers: Vec<Browser>) -> Self {
        AppState {
            dirs,
            caps: PlatformCaps {
                source_app_rules: false,
            },
            config: Mutex::new(config),
            browsers: Mutex::new(browsers),
            prompt: PromptState::new(),
            primary_targets: Mutex::new(Vec::new()),
            primary_items: Mutex::new(Vec::new()),
            icon_cache: Mutex::new(HashMap::new()),
            save_lock: Mutex::new(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{OpenIn, Rule};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-state-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(target_os = "linux")]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: Vec::new(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
        }
    }

    /// A minimal `AppState` pointed at a disposable temp dir, so these
    /// tests stay hermetic.
    fn test_state(root: &std::path::Path) -> AppState {
        AppState::for_test(test_dirs(root), Config::default(), Vec::new())
    }

    fn target(browser: &str) -> Target {
        Target {
            browser: browser.to_string(),
            profile: None,
            private: false,
        }
    }

    #[test]
    fn update_config_preserves_other_fields() {
        let root = temp_root("preserve");
        let state = test_state(&root);
        {
            let mut cfg = state.config.lock().unwrap();
            cfg.shown_browsers = vec!["chrome".to_string(), "firefox".to_string()];
            cfg.rules.push(Rule {
                name: "work".to_string(),
                preset: None,
                match_patterns: vec!["company.com".to_string()],
                source_apps: None,
                open_in: OpenIn::Target(target("firefox")),
            });
        }

        state
            .update_config(|cfg| cfg.primary = target("chrome"))
            .expect("update_config");

        let cfg = state.config.lock().unwrap();
        assert_eq!(cfg.primary, target("chrome"));
        assert_eq!(
            cfg.shown_browsers,
            vec!["chrome".to_string(), "firefox".to_string()]
        );
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(cfg.rules[0].name, "work");
        fs_cleanup(&root);
    }

    #[test]
    fn update_config_preserves_fields_under_concurrent_updates() {
        let root = temp_root("concurrent");
        let state = Arc::new(test_state(&root));
        {
            let mut cfg = state.config.lock().unwrap();
            cfg.shown_browsers = vec!["chrome".to_string(), "firefox".to_string()];
        }

        let handles: Vec<_> = (0..12)
            .map(|i| {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    state.update_config(move |cfg| cfg.primary = target(&format!("browser-{i}")))
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .expect("thread panicked")
                .expect("update_config failed");
        }

        let cfg = state.config.lock().unwrap();
        assert_eq!(
            cfg.shown_browsers,
            vec!["chrome".to_string(), "firefox".to_string()],
            "a field untouched by any update_config call must survive every concurrent save"
        );
        assert!(cfg.primary.browser.starts_with("browser-"));

        // The on-disk file must agree with whichever update won.
        let on_disk = app_config::load_or_init(&state.dirs.config_path(), None, &[]).0;
        assert_eq!(on_disk, *cfg);
        fs_cleanup(&root);
    }

    fn fs_cleanup(root: &std::path::Path) {
        let _ = std::fs::remove_dir_all(root);
    }
}
