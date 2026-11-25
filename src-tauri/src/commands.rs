//! The Tauri command contract between the Rust backend and the UI.
//!
//! This file freezes the command *signatures* (names, argument and return
//! types) so the backend (owns the rest of `src-tauri/**`) and the UI
//! (owns `ui/**`) can be built in parallel. The types below are
//! re-exported from `linh-core` and mirrored by hand in `ui/src/api.ts` —
//! keep both in lockstep when this contract changes.
//!
//! Extra parameters (`tauri::State`, `tauri::AppHandle`) are injected by
//! Tauri itself and are invisible to the JS `invoke()` call, so adding them
//! here does not change the contract the UI calls against.

use std::time::Instant;

use linh_core::{Browser, Config, PlatformCaps, Preset, Target};
use tauri::{AppHandle, State};

use crate::platform::current::{apps, default_browser, icons};
use crate::platform::logging;
use crate::routing;
use crate::state::AppState;
use crate::tray;

/// Lists the browsers detected on this system.
#[tauri::command]
pub fn list_browsers(state: State<AppState>) -> Vec<Browser> {
    state.refresh_browsers()
}

/// Returns the current configuration (loaded from `linh.toml`, or the
/// default config if none exists yet / it failed to parse).
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

/// Persists `config` to `linh.toml`.
#[tauri::command]
pub fn save_config(config: Config, state: State<AppState>) -> Result<(), String> {
    state.persist_config(config)?;
    tray::sync_checked_state(&state);
    Ok(())
}

/// Returns this platform's routing capabilities (e.g. whether source-app
/// rules are supported). Linux: `source_app_rules` is always `false`.
#[tauri::command]
pub fn get_platform_caps(state: State<AppState>) -> PlatformCaps {
    state.caps
}

/// Returns whether Linh is currently registered as the default browser.
#[tauri::command]
pub fn is_default_browser() -> bool {
    default_browser::is_default_browser()
}

/// Registers Linh as the default browser for `http`/`https`.
#[tauri::command]
pub fn set_default_browser() -> Result<(), String> {
    default_browser::set_default_browser()
}

/// Returns the URL the prompt window is currently showing, if any.
#[tauri::command]
pub fn get_prompt_url(state: State<AppState>) -> Option<String> {
    state.prompt.pending_url()
}

/// The user picked `target` in the prompt window for the pending URL.
/// Taking the pending URL and hiding the prompt happen atomically (see
/// [`crate::prompt_state::PromptState`]), so there's no window where the
/// URL is gone but the prompt is still showing, or vice versa. Logs
/// `choose->spawn`, timed from this call's entry (the moment the human
/// made their choice) — the time spent waiting for that choice is
/// deliberately not counted.
#[tauri::command]
pub fn prompt_choose(target: Target, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let chosen_at = Instant::now();
    let url = state
        .prompt
        .take_and_hide(&app)
        .ok_or_else(|| "no pending URL".to_string())?;
    let result = routing::open_target(&state, &target, &url);
    match &result {
        Ok(()) => logging::log_line(
            &state.dirs,
            &format!(
                "prompt choice {url} -> {target:?} (choose->spawn {}ms)",
                chosen_at.elapsed().as_millis()
            ),
        ),
        Err(err) => logging::log_line(
            &state.dirs,
            &format!("failed to open {url} from prompt: {err}"),
        ),
    }
    result
}

/// The user cancelled the prompt (e.g. pressed `Esc`); the pending URL is
/// dropped without opening anything. Clearing the URL and hiding the
/// prompt happen atomically, same as [`prompt_choose`].
#[tauri::command]
pub fn prompt_cancel(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    state.prompt.take_and_hide(&app);
    Ok(())
}

/// Lists the four built-in rule presets, with `app_installed` filled in
/// against the apps currently detected on this system.
#[tauri::command]
pub fn list_presets(state: State<AppState>) -> Vec<Preset> {
    linh_core::PRESET_DEFS
        .iter()
        .map(|def| Preset {
            id: def.id.to_string(),
            name: def.name.to_string(),
            patterns: def.patterns.iter().map(|p| p.to_string()).collect(),
            app: def.app.map(str::to_string),
            app_installed: def
                .app
                .is_some_and(|app| apps::is_app_installed(&state.dirs, app)),
        })
        .collect()
}

/// Resolves `icon` (an absolute path, or a freedesktop icon name) to a
/// `data:` URL, memoized for the rest of this session.
#[tauri::command]
pub fn get_icon(icon: String, state: State<AppState>) -> Option<String> {
    if let Some(cached) = state.icon_cache.lock().unwrap().get(&icon) {
        return cached.clone();
    }
    let result = icons::resolve_icon(&state.dirs, &icon);
    state
        .icon_cache
        .lock()
        .unwrap()
        .insert(icon, result.clone());
    result
}
