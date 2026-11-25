//! Lazily creates the `settings` webview on first demand (tkhieu/linh#15
//! step 1) instead of at startup, so a fresh launch only ever pays for
//! `prompt`'s `WebKitWebProcess`. The window it builds shares that same
//! process (`with_related_view`, Linux only): WebKit tears down a web
//! process when its last related view closes, and `Window::destroy()`
//! can't be used to reclaim it anyway (wry 0.57.1's IPC handler closure
//! holds a strong `WebView` clone, keeping it alive regardless), so
//! closing Settings only ever hides it — the window, and the process it
//! shares with `prompt`, both stay alive for an instant reopen.

use std::time::Instant;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::allocator;
use crate::mem_profile;
use crate::platform::current::dirs::Dirs;
use crate::platform::logging;
use crate::routing;

pub const LABEL: &str = "settings";

/// Shows the `settings` window, creating it first if this is the first
/// time it has been requested. Safe to call from any thread — the
/// actual show-or-create decision and all window/webview work always run
/// on the main thread, via [`AppHandle::run_on_main_thread`], which is
/// also how Tauri's single GTK event loop serializes two calls that race
/// in from different threads (e.g. two quick tray clicks): both get
/// queued, but only ever run one at a time, so the second one's
/// [`should_create`] check always sees whatever the first one already
/// did.
pub fn show_or_create(app: &AppHandle, dirs: &Dirs) {
    let app_for_main = app.clone();
    let dirs_for_main = dirs.clone();
    let dirs_for_err = dirs.clone();
    let scheduled = app.run_on_main_thread(move || {
        if should_create(app_for_main.get_webview_window(LABEL).is_some()) {
            create(&app_for_main, &dirs_for_main);
        } else {
            routing::show_and_focus(&app_for_main, LABEL);
        }
    });
    if let Err(err) = scheduled {
        logging::log_line(
            &dirs_for_err,
            &format!("settings: failed to schedule show/create: {err}"),
        );
    }
}

/// Whether a new `settings` webview still needs to be created, given
/// whether one is already managed. Pure (no Tauri/GTK types), so it is
/// directly unit-testable without a running app.
fn should_create(window_exists: bool) -> bool {
    !window_exists
}

/// Builds the `settings` window, related to `prompt`'s webview when it
/// is available (sharing its `WebKitWebProcess`) and logs + falls back to
/// an unrelated window otherwise, then always installs the close-hides
/// handler. Must run on the main thread. Linux only: sharing a related
/// webview is a `webkit2gtk`-specific workaround (see this module's doc
/// comment); other platforms always build standalone.
#[cfg(target_os = "linux")]
fn create(app: &AppHandle, dirs: &Dirs) {
    mem_profile::record_settings_create_requested(Instant::now());

    match app.get_webview_window("prompt") {
        Some(prompt_window) => {
            let app_for_build = app.clone();
            let dirs_for_build = dirs.clone();
            let dirs_for_err = dirs.clone();
            // Runs synchronously here: `with_webview` dispatches through
            // the same main-thread-aware mechanism as
            // `run_on_main_thread`, so calling it from a thread already
            // on the main loop (as `create` always is) executes the
            // closure immediately in place rather than deadlocking or
            // deferring it.
            let scheduled = prompt_window.with_webview(move |webview| {
                build(&app_for_build, &dirs_for_build, Some(webview.inner()));
            });
            if let Err(err) = scheduled {
                logging::log_line(
                    &dirs_for_err,
                    &format!(
                        "settings: with_webview failed, creating without a related view: {err}"
                    ),
                );
                build(app, dirs, None);
            }
        }
        None => {
            logging::log_line(
                dirs,
                "settings: prompt window missing, creating without a related view",
            );
            build(app, dirs, None);
        }
    }
}

// ported in a later change: no related-webview sharing outside Linux
// yet, so Settings always builds standalone there.
#[cfg(not(target_os = "linux"))]
fn create(app: &AppHandle, dirs: &Dirs) {
    mem_profile::record_settings_create_requested(Instant::now());
    build(app, dirs);
}

/// Actually builds and manages the `settings` window (matching the
/// properties it used to have in `tauri.conf.json`) and wires its
/// close button to hide instead of destroy. Must run on the main thread.
#[cfg(target_os = "linux")]
fn build(app: &AppHandle, dirs: &Dirs, related_view: Option<webkit2gtk::WebView>) {
    let mut builder =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
            .title("Linh Settings")
            .inner_size(1040.0, 620.0)
            .min_inner_size(860.0, 520.0)
            .resizable(true)
            .decorations(true);
    if let Some(related_view) = related_view {
        builder = builder.with_related_view(related_view);
    }

    match builder.build() {
        Ok(window) => install_close_handler(app, window),
        Err(err) => {
            logging::log_line(dirs, &format!("settings: failed to build window: {err}"));
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn build(app: &AppHandle, dirs: &Dirs) {
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
        .title("Linh Settings")
        .inner_size(1040.0, 620.0)
        .min_inner_size(860.0, 520.0)
        .resizable(true)
        .decorations(true);

    match builder.build() {
        Ok(window) => install_close_handler(app, window),
        Err(err) => {
            logging::log_line(dirs, &format!("settings: failed to build window: {err}"));
        }
    }
}

fn install_close_handler(app: &AppHandle, window: tauri::WebviewWindow) {
    let app_for_close = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Some(window) = app_for_close.get_webview_window(LABEL) {
                if window.hide().is_ok() {
                    allocator::trim();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_when_no_window_exists_yet() {
        assert!(should_create(false));
    }

    #[test]
    fn does_not_create_when_a_window_already_exists() {
        assert!(!should_create(true));
    }
}
