//! `linh.toml` load/save. The file format and atomic-write logic are
//! OS-agnostic and already fully working; only resolving a *previous*
//! system default browser (via [`super::default_browser::matches_desktop_id`],
//! not yet implemented on macOS) is a stub for now.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use linh_core::{Browser, Config, Target};

use super::default_browser;

static SAVE_LOCK: Mutex<()> = Mutex::new(());
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigStatus {
    Loaded,
    Created,
    Broken,
}

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
            let _ = write_atomic(path, &config);
            (config, ConfigStatus::Created)
        }
        Err(_) => (fallback_config(detected), ConfigStatus::Broken),
    }
}

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
        .and_then(|id| {
            detected
                .iter()
                .find(|b| default_browser::matches_desktop_id(b, id))
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
