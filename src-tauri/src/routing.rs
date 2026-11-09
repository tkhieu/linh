use std::time::Instant;

use linh_core::{AppTarget, Decision, LaunchCommand, Target};
use tauri::{AppHandle, Manager};
use url::Url;

use crate::platform::linux::{apps, launcher, logging, url_intake};
use crate::state::AppState;

/// Decides where `raw_url` should go (per `linh_core::decide`, with Linux
/// capabilities) and either spawns the target browser or shows the prompt
/// window. Non-`http`/`https` URLs are ignored (logged).
///
/// `received_at` is when the URL was actually received — the top of
/// `main` for the first launch (so cold-start scanning/config-load time
/// is counted honestly) or the moment the single-instance callback fired
/// for a later launch — not when this function happens to run. Logs
/// `receive->spawn` when a browser is spawned directly. When the prompt
/// is shown instead, `receive->prompt_shown` is logged by
/// [`crate::prompt_state::PromptState`]'s main-thread reconcile, right
/// after the window actually shows (not here, at schedule time) — the
/// time a human then spends choosing is not part of either.
pub fn route_url(app: &AppHandle, state: &AppState, raw_url: &str, received_at: Instant) {
    let Some(url) = url_intake::filter_routable_url(raw_url) else {
        logging::log_line(
            &state.dirs,
            &format!("ignoring non-routable URL argument: {raw_url}"),
        );
        return;
    };
    let Ok(parsed) = Url::parse(&url) else {
        logging::log_line(
            &state.dirs,
            &format!("failed to re-parse routed URL: {url}"),
        );
        return;
    };

    let decision = {
        let config = state.config.lock().unwrap();
        linh_core::decide(&config, &parsed, state.caps)
    };

    match decision {
        Decision::Open(target) => {
            if let Err(err) = open_target(state, &target, &url) {
                logging::log_line(&state.dirs, &format!("failed to open {url}: {err}"));
            } else {
                logging::log_line(
                    &state.dirs,
                    &format!(
                        "routed {url} -> {target:?} (receive->spawn {}ms)",
                        received_at.elapsed().as_millis()
                    ),
                );
            }
        }
        Decision::OpenApp(app_target) => {
            if let Err(err) = open_app_target(state, &app_target, &url) {
                logging::log_line(&state.dirs, &format!("failed to open {url}: {err}"));
            } else {
                logging::log_line(
                    &state.dirs,
                    &format!(
                        "routed {url} -> app {app_target:?} (receive->spawn {}ms)",
                        received_at.elapsed().as_millis()
                    ),
                );
            }
        }
        Decision::Prompt => {
            state.prompt.show(app, url, received_at);
        }
    }
}

/// Resolves `target` against the detected browsers and spawns it, never
/// through a shell.
pub fn open_target(state: &AppState, target: &Target, url: &str) -> Result<(), String> {
    let browsers = state.browsers.lock().unwrap();
    let browser = browsers
        .iter()
        .find(|b| b.id == target.browser)
        .ok_or_else(|| format!("browser '{}' is not installed", target.browser))?;
    let cmd = linh_core::build_launch_command(browser, target, url).map_err(|e| e.to_string())?;
    drop(browsers);
    launcher::spawn_detached(&cmd).map_err(|e| e.to_string())
}

/// Resolves `app_target` for `url`: launches the app (never through a
/// shell) when it's installed and `url` can be converted for it;
/// otherwise — app not installed, URL not convertible, or the spawn
/// itself fails — opens `app_target.fallback`, or `Config::primary` when
/// there is no fallback, with the original (unconverted) `url`. Returns
/// `Err` only if that fallback open also fails.
pub fn open_app_target(state: &AppState, app_target: &AppTarget, url: &str) -> Result<(), String> {
    if let Some(cmd) = resolve_app_launch(state, app_target, url) {
        if launcher::spawn_detached(&cmd).is_ok() {
            return Ok(());
        }
    }
    let fallback = app_target
        .fallback
        .clone()
        .unwrap_or_else(|| state.config.lock().unwrap().primary.clone());
    open_target(state, &fallback, url)
}

/// Builds the launch command for `app_target.app`, or `None` if the URL
/// isn't convertible for that app or the app isn't installed.
fn resolve_app_launch(
    state: &AppState,
    app_target: &AppTarget,
    url: &str,
) -> Option<LaunchCommand> {
    let launch_url = match app_target.app.as_str() {
        "zoom" => linh_core::zoom_app_url(url)?,
        "teams" => url.to_string(),
        _ => return None,
    };
    let app = apps::find_app(&state.dirs, &app_target.app)?;
    let dummy_target = Target {
        browser: app_target.app.clone(),
        profile: None,
        private: false,
    };
    linh_core::build_launch_command(&app, &dummy_target, &launch_url).ok()
}

pub fn show_and_focus(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
