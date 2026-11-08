//! The Tauri command contract between the Rust backend and the UI.
//!
//! This file freezes the command *signatures* (names, argument and return
//! types) so phase 2 (backend, owns the rest of `src-tauri/**`) and phase 3
//! (UI, owns `ui/**`) can be built in parallel. The types below are
//! re-exported from `linh-core` and mirrored by hand in `ui/src/api.ts` —
//! keep both in lockstep when this contract changes.
//!
//! Bodies here are stubs: they return sensible defaults (an empty list, the
//! default config, `false`, `None`) or `Err("not implemented")`. Phase 2
//! replaces the bodies with real platform logic; the signatures should not
//! need to change.

use linh_core::{Browser, Config, PlatformCaps, Target};

/// Lists the browsers detected on this system.
#[tauri::command]
pub fn list_browsers() -> Vec<Browser> {
    Vec::new()
}

/// Returns the current configuration (loaded from `linh.toml`, or the
/// default config if none exists yet / it failed to parse).
#[tauri::command]
pub fn get_config() -> Config {
    Config::default()
}

/// Persists `config` to `linh.toml`.
#[tauri::command]
pub fn save_config(config: Config) -> Result<(), String> {
    let _ = config;
    Err("not implemented".to_string())
}

/// Returns this platform's routing capabilities (e.g. whether source-app
/// rules are supported). Linux: `source_app_rules` is always `false`.
#[tauri::command]
pub fn get_platform_caps() -> PlatformCaps {
    PlatformCaps {
        source_app_rules: false,
    }
}

/// Returns whether Linh is currently registered as the default browser.
#[tauri::command]
pub fn is_default_browser() -> bool {
    false
}

/// Registers Linh as the default browser for `http`/`https`.
#[tauri::command]
pub fn set_default_browser() -> Result<(), String> {
    Err("not implemented".to_string())
}

/// Returns the URL the prompt window is currently showing, if any.
#[tauri::command]
pub fn get_prompt_url() -> Option<String> {
    None
}

/// The user picked `target` in the prompt window for the pending URL.
#[tauri::command]
pub fn prompt_choose(target: Target) -> Result<(), String> {
    let _ = target;
    Err("not implemented".to_string())
}

/// The user cancelled the prompt (e.g. pressed `Esc`); the pending URL is
/// dropped without opening anything.
#[tauri::command]
pub fn prompt_cancel() -> Result<(), String> {
    Err("not implemented".to_string())
}
