use std::sync::Mutex;

use linh_core::{Browser, Config, PlatformCaps, Target};
use tauri::menu::CheckMenuItem;
use tauri::Wry;

use crate::platform::linux::{app_config, browsers, default_browser, dirs::Dirs, notify};

/// Shared, mutable application state, managed by Tauri and reachable from
/// every command and from the tray menu's click handler.
pub struct AppState {
    pub dirs: Dirs,
    pub caps: PlatformCaps,
    pub config: Mutex<Config>,
    pub browsers: Mutex<Vec<Browser>>,
    pub pending_url: Mutex<Option<String>>,
    /// The targets listed in the tray's "Primary Browser" submenu, indexed
    /// by the `primary:<index>` menu item id that was built from them.
    pub primary_targets: Mutex<Vec<Target>>,
    /// The live `CheckMenuItem` handles, same order as `primary_targets`,
    /// used to update which one is checked without replacing the tray's
    /// menu object (unsupported once set, on Linux).
    pub primary_items: Mutex<Vec<CheckMenuItem<Wry>>>,
    /// Serializes the whole "write the file, then update the in-memory
    /// copy" sequence in [`AppState::persist_config`], so two concurrent
    /// callers (the tray's click handler and a `save_config` command)
    /// can never leave `config` and the on-disk file pointing at
    /// different saves.
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
            pending_url: Mutex::new(None),
            primary_targets: Mutex::new(Vec::new()),
            primary_items: Mutex::new(Vec::new()),
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

    /// Validates and atomically persists `config`, then updates the
    /// in-memory copy on success. The whole sequence is serialized via
    /// `save_lock` so the file and the in-memory copy can never end up
    /// reflecting two different concurrent saves.
    pub fn persist_config(&self, config: Config) -> Result<(), String> {
        let _guard = self.save_lock.lock().unwrap();
        app_config::save(&self.dirs.config_path(), &config)?;
        *self.config.lock().unwrap() = config;
        Ok(())
    }
}
