//! The Tauri command contract between the Rust backend and the UI.
//!
//! This file freezes the command *signatures* (names, argument and return
//! types) so phase 2 (backend, owns the rest of `src-tauri/**`) and phase 3
//! (UI, owns `ui/**`) can be built in parallel. The types below are
//! re-exported from `linh-core` and mirrored by hand in `ui/src/api.ts` —
//! keep both in lockstep when this contract changes.
//!
//! Extra parameters (`tauri::State`, `tauri::AppHandle`) are injected by
//! Tauri itself and are invisible to the JS `invoke()` call, so adding them
//! here does not change the contract the UI calls against.

use std::time::Instant;

use linh_core::{Browser, Config, PlatformCaps, Target};
use tauri::{AppHandle, Manager, State};

use crate::platform::linux::{default_browser, logging};
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
    state.pending_url.lock().unwrap().clone()
}

/// The user picked `target` in the prompt window for the pending URL.
/// Logs `choose->spawn`, timed from this call's entry (the moment the
/// human made their choice) — the time spent waiting for that choice is
/// deliberately not counted.
#[tauri::command]
pub fn prompt_choose(target: Target, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let chosen_at = Instant::now();
    let url = state
        .pending_url
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "no pending URL".to_string())?;
    let result = routing::open_target(&state, &target, &url);
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
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
/// dropped without opening anything.
#[tauri::command]
pub fn prompt_cancel(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    *state.pending_url.lock().unwrap() = None;
    if let Some(window) = app.get_webview_window("prompt") {
        let _ = window.hide();
    }
    Ok(())
}
