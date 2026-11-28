use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use linh_core::{Browser, Config, PlatformCaps, Target};
use tauri::menu::CheckMenuItem;
use tauri::Wry;

use crate::platform::current::{app_config, browsers, default_browser, dirs::Dirs, notify};
use crate::prompt_state::PromptState;
use crate::updates;
use crate::updates::Dist;

/// Maps this build's `LINH_DIST` to the `PlatformCaps` it implies.
/// Deliberately pure and parameterized by `dist` rather than reading
/// `updates::current_dist()` itself, so it's unit-tested directly on
/// every CI runner — not just the one real macOS/`LINH_DIST=mas` build
/// that ever reaches `Dist::Mas` at runtime.
pub(crate) fn platform_caps_for_dist(dist: Dist) -> PlatformCaps {
    PlatformCaps {
        source_app_rules: false,
        restrict_profile_private: dist == Dist::Mas,
    }
}

/// Shared, mutable application state, managed by Tauri and reachable from
/// every command and from the tray menu's click handler.
pub struct AppState {
    pub dirs: Dirs,
    pub caps: PlatformCaps,
    pub config: Mutex<Config>,
    pub browsers: Mutex<Vec<Browser>>,
    /// Current Linux desktop-id aliases; empty on other platforms.
    pub browser_aliases: Mutex<HashMap<String, String>>,
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
    /// Background update checking and in-place install state (see
    /// [`crate::updates`]).
    pub updates: updates::Updates,
    /// Set the first time this run shows the "profiles and private
    /// windows aren't available" notice (only ever true when
    /// `caps.restrict_profile_private` is), so a config full of rules
    /// that each name a profile/private target notifies once per run
    /// instead of once per routed link. See
    /// `routing::degrade_target`/`routing::notify_profile_private_unsupported_once`.
    /// `Arc`-wrapped (unlike the other `Mutex`-guarded fields above) so
    /// `routing::run_watched_fallback`'s worker-thread closure can hold
    /// its own clone the same way it already snapshots `dirs`/`browsers`,
    /// rather than needing a live `&AppState` or `AppHandle` it doesn't
    /// otherwise have this deep into an async outcome handler.
    pub profile_private_notice_shown: Arc<AtomicBool>,
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
        #[cfg(target_os = "windows")]
        default_browser::refresh_registration_if_stale(&dirs);
        #[cfg(target_os = "linux")]
        let discovery = browsers::detect_browsers_and_aliases(&dirs);
        #[cfg(target_os = "linux")]
        let detected = discovery.browsers;
        #[cfg(not(target_os = "linux"))]
        let detected = browsers::detect_browsers(&dirs);
        let previous_default = default_browser::current_default_browser_id();
        let config_path = dirs.config_path();
        #[allow(unused_mut)] // Linux applies desktop-id aliases after loading.
        let (mut config, status) =
            app_config::load_or_init(&config_path, previous_default.as_deref(), &detected);
        #[cfg(target_os = "linux")]
        if status == app_config::ConfigStatus::Loaded {
            app_config::migrate_browser_aliases(&mut config, &discovery.aliases, &dirs);
        }

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
            caps: platform_caps_for_dist(updates::current_dist()),
            config: Mutex::new(config),
            browsers: Mutex::new(detected),
            #[cfg(target_os = "linux")]
            browser_aliases: Mutex::new(discovery.aliases),
            #[cfg(not(target_os = "linux"))]
            browser_aliases: Mutex::new(HashMap::new()),
            prompt: PromptState::new(),
            primary_targets: Mutex::new(Vec::new()),
            primary_items: Mutex::new(Vec::new()),
            icon_cache: Mutex::new(HashMap::new()),
            updates: updates::Updates::init(),
            profile_private_notice_shown: Arc::new(AtomicBool::new(false)),
            save_lock: Mutex::new(()),
        }
    }

    /// Re-scans installed browsers, updates the cache, and returns the
    /// fresh list.
    pub fn refresh_browsers(&self) -> Vec<Browser> {
        #[cfg(target_os = "linux")]
        {
            let discovery = browsers::detect_browsers_and_aliases(&self.dirs);
            let detected = discovery.browsers;
            let _save_guard = self.save_lock.lock().unwrap();
            // Publish the browser list and its aliases together. Routing
            // locks browsers before aliases, so it sees a matching pair.
            let mut current_browsers = self.browsers.lock().unwrap();
            *self.browser_aliases.lock().unwrap() = discovery.aliases;
            *current_browsers = detected.clone();
            drop(current_browsers);
            let aliases = self.browser_aliases.lock().unwrap();
            if !aliases.is_empty() {
                let mut config = self.config.lock().unwrap();
                app_config::migrate_browser_aliases(&mut config, &aliases, &self.dirs);
            }
            detected
        }
        #[cfg(not(target_os = "linux"))]
        {
            let detected = browsers::detect_browsers(&self.dirs);
            *self.browsers.lock().unwrap() = detected.clone();
            detected
        }
    }

    /// Applies `mutate` to a clone of the *current* in-memory config,
    /// then validates and persists the result and updates the in-memory
    /// copy — reading, mutating, saving, and updating memory all under
    /// `save_lock`, so no other save can read a stale config out from
    /// under `mutate`, and no field `mutate` doesn't touch is ever lost.
    /// Used by the tray, which only ever changes `primary`, and by
    /// [`AppState::persist_config`] for a whole-config replacement from
    /// the UI.
    ///
    /// Degrades `config.primary` (via `routing::degrade_target`) right
    /// after `mutate` runs, before saving: `tray::primary_entries` never
    /// *offers* a profile/private entry under `caps.restrict_profile_private`,
    /// but `mutate` can still be the Settings UI's `save_config` command
    /// handing over a whole `Config` it built from a stale target, or one
    /// from a channel this build wasn't on when it was first saved — in
    /// either case, persisting an "impossible" primary that can never
    /// actually launch as written would be a silent lie the config file
    /// and the Settings UI both keep telling until someone notices.
    pub fn update_config(&self, mutate: impl FnOnce(&mut Config)) -> Result<(), String> {
        let _guard = self.save_lock.lock().unwrap();
        let mut config = self.config.lock().unwrap().clone();
        mutate(&mut config);
        #[cfg(target_os = "linux")]
        app_config::migrate_browser_aliases(
            &mut config,
            &self.browser_aliases.lock().unwrap(),
            &self.dirs,
        );
        if let Some(degraded) = crate::routing::degrade_target(self.caps, &config.primary) {
            config.primary = degraded;
        }
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
        Self::for_test_with_caps(
            dirs,
            config,
            browsers,
            PlatformCaps {
                source_app_rules: false,
                restrict_profile_private: false,
            },
        )
    }

    /// Like [`AppState::for_test`], but with an explicit `caps` — used by
    /// tests that need `restrict_profile_private: true` without a real
    /// `LINH_DIST=mas` build (not possible on this Linux CI host at all,
    /// since that's baked in at compile time).
    #[cfg(test)]
    pub(crate) fn for_test_with_caps(
        dirs: Dirs,
        config: Config,
        browsers: Vec<Browser>,
        caps: PlatformCaps,
    ) -> Self {
        AppState {
            dirs,
            caps,
            config: Mutex::new(config),
            browsers: Mutex::new(browsers),
            browser_aliases: Mutex::new(HashMap::new()),
            prompt: PromptState::new(),
            primary_targets: Mutex::new(Vec::new()),
            primary_items: Mutex::new(Vec::new()),
            icon_cache: Mutex::new(HashMap::new()),
            updates: updates::Updates::init(),
            profile_private_notice_shown: Arc::new(AtomicBool::new(false)),
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

    #[cfg(target_os = "linux")]
    fn write_browser_entry(dir: &std::path::Path, id: &str, no_display: bool) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.desktop")),
            format!(
                "[Desktop Entry]\nName=Chrome\nExec=/usr/bin/google-chrome-stable %U\nMimeType=x-scheme-handler/http;\nNoDisplay={no_display}\n"
            ),
        )
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn refresh_migrates_new_aliases_in_memory_without_saving() {
        let root = temp_root("refresh-aliases");
        let apps = root.join("apps");
        write_browser_entry(&apps, "google-chrome", true);
        write_browser_entry(&apps, "com.google.Chrome", false);
        let mut dirs = test_dirs(&root);
        dirs.application_dirs = vec![apps];
        let initial = browsers::detect_browsers_and_aliases(&dirs);
        assert_eq!(initial.browsers[0].id, "com.google.Chrome");
        let config = Config {
            primary: target("com.google.Chrome"),
            shown_browsers: vec!["com.google.Chrome/Default#private".into()],
            rules: vec![Rule {
                name: "work".into(),
                preset: None,
                match_patterns: vec!["example.com".into()],
                source_apps: None,
                open_in: OpenIn::Target(target("com.google.Chrome")),
            }],
            ..Config::default()
        };
        let state = AppState::for_test(dirs, config, initial.browsers);
        *state.browser_aliases.lock().unwrap() = initial.aliases;
        write_browser_entry(&state.dirs.application_dirs[0], "google-chrome", false);
        write_browser_entry(&state.dirs.application_dirs[0], "com.google.Chrome", true);
        let detected = state.refresh_browsers();
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].id, "google-chrome");
        let migrated = state.config.lock().unwrap().clone();
        assert_eq!(migrated.primary.browser, "google-chrome");
        assert_eq!(
            migrated.shown_browsers,
            vec!["google-chrome/Default#private"]
        );
        assert!(
            matches!(&migrated.rules[0].open_in, OpenIn::Target(t) if t.browser == "google-chrome")
        );
        assert_eq!(
            state
                .browser_aliases
                .lock()
                .unwrap()
                .get("com.google.Chrome")
                .map(String::as_str),
            Some("google-chrome")
        );
        state.refresh_browsers();
        assert_eq!(*state.config.lock().unwrap(), migrated);
        assert!(!state.dirs.config_path().exists());
        fs_cleanup(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn refresh_without_alias_change_leaves_config_untouched() {
        let root = temp_root("refresh-no-change");
        let apps = root.join("apps");
        write_browser_entry(&apps, "google-chrome", false);
        let mut dirs = test_dirs(&root);
        dirs.application_dirs = vec![apps];
        let config = Config {
            primary: target("google-chrome"),
            shown_browsers: vec!["google-chrome".into(), "google-chrome".into()],
            ..Config::default()
        };
        let state = AppState::for_test(dirs, config.clone(), Vec::new());
        state.refresh_browsers();
        state.refresh_browsers();
        assert_eq!(*state.config.lock().unwrap(), config);
        assert!(state.browser_aliases.lock().unwrap().is_empty());
        assert!(!state.dirs.config_path().exists());
        fs_cleanup(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn saving_a_stale_settings_draft_rewrites_current_aliases() {
        let root = temp_root("save-stale-draft");
        let state = test_state(&root);
        state
            .browser_aliases
            .lock()
            .unwrap()
            .insert("com.google.Chrome".into(), "google-chrome".into());
        let stale = Config {
            primary: target("com.google.Chrome"),
            shown_browsers: vec!["com.google.Chrome#private".into()],
            ..Config::default()
        };
        state.persist_config(stale).unwrap();
        let saved = app_config::load_or_init(&state.dirs.config_path(), None, &[]).0;
        assert_eq!(saved.primary.browser, "google-chrome");
        assert_eq!(saved.shown_browsers, vec!["google-chrome#private"]);
        assert_eq!(*state.config.lock().unwrap(), saved);
        fs_cleanup(&root);
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

    #[test]
    fn only_mas_restricts_profile_and_private() {
        for dist in [
            Dist::Github,
            Dist::Msstore,
            Dist::Flathub,
            Dist::Copr,
            Dist::Ppa,
        ] {
            assert!(
                !platform_caps_for_dist(dist).restrict_profile_private,
                "{dist:?} must not restrict profile/private targets"
            );
        }
        assert!(platform_caps_for_dist(Dist::Mas).restrict_profile_private);
    }

    #[test]
    fn mas_capability_mapping_leaves_source_app_rules_untouched() {
        assert!(!platform_caps_for_dist(Dist::Mas).source_app_rules);
    }

    #[test]
    fn update_config_degrades_a_profile_or_private_primary_under_mas_caps() {
        let root = temp_root("degrade-primary");
        let caps = platform_caps_for_dist(Dist::Mas);
        let state =
            AppState::for_test_with_caps(test_dirs(&root), Config::default(), Vec::new(), caps);

        state
            .update_config(|cfg| {
                cfg.primary = Target {
                    browser: "chrome".to_string(),
                    profile: Some("Work".to_string()),
                    private: true,
                }
            })
            .expect("update_config");

        let cfg = state.config.lock().unwrap();
        assert_eq!(
            cfg.primary,
            Target {
                browser: "chrome".to_string(),
                profile: None,
                private: false,
            },
            "a profile/private primary must be degraded before it's ever persisted under MAS caps"
        );

        let on_disk = app_config::load_or_init(&state.dirs.config_path(), None, &[]).0;
        assert_eq!(
            on_disk.primary, cfg.primary,
            "the degraded primary must be what actually hit disk, not just the in-memory copy"
        );
        fs_cleanup(&root);
    }

    #[test]
    fn update_config_leaves_a_profile_or_private_primary_alone_without_mas_caps() {
        let root = temp_root("no-degrade-primary");
        let state = test_state(&root);

        state
            .update_config(|cfg| {
                cfg.primary = Target {
                    browser: "chrome".to_string(),
                    profile: Some("Work".to_string()),
                    private: false,
                }
            })
            .expect("update_config");

        assert_eq!(
            state.config.lock().unwrap().primary,
            Target {
                browser: "chrome".to_string(),
                profile: Some("Work".to_string()),
                private: false,
            }
        );
        fs_cleanup(&root);
    }
}
