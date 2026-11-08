#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod platform;
mod routing;
mod state;
mod tray;

use std::sync::Mutex;
use std::time::Instant;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use platform::linux::url_intake;
use state::AppState;

/// Argv (with the moment it was received) queued by the single-instance
/// callback when it fired before `setup` had `app.manage`d `AppState` yet
/// — a real race between plugin registration and `setup` completing, not
/// a hypothetical one. `setup` drains this once `AppState` is ready.
#[derive(Debug, Default)]
struct PendingLaunches(Vec<(Instant, Vec<String>)>);

impl PendingLaunches {
    fn enqueue(&mut self, received_at: Instant, args: Vec<String>) {
        self.0.push((received_at, args));
    }

    /// Removes and returns every pending launch, oldest first.
    fn drain(&mut self) -> Vec<(Instant, Vec<String>)> {
        std::mem::take(&mut self.0)
    }
}

static PENDING_LAUNCHES: Mutex<PendingLaunches> = Mutex::new(PendingLaunches(Vec::new()));

/// Routes `args`' first `http`/`https` URL (logged, timed from
/// `received_at`), or shows the settings window if there is none.
fn dispatch_launch(app: &AppHandle, state: &AppState, args: &[String], received_at: Instant) {
    match url_intake::first_routable_url(args) {
        Some(url) => routing::route_url(app, state, &url, received_at),
        None => routing::show_and_focus(app, "settings"),
    }
}

fn main() {
    // Captured before anything else so first-launch latency logging
    // honestly includes browser scanning and config load, not just the
    // final routing decision.
    let process_start = Instant::now();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // The moment this already-running instance was actually
            // notified of the second launch's argv.
            let received_at = Instant::now();
            match app.try_state::<AppState>() {
                Some(state) => dispatch_launch(app, &state, &args, received_at),
                // setup() hasn't finished managing AppState yet (the
                // plugin can fire before setup completes); queue this
                // launch and let setup() drain it once ready, rather
                // than panicking and dropping the URL.
                None => PENDING_LAUNCHES.lock().unwrap().enqueue(received_at, args),
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            let state = AppState::init();
            app.manage(state);
            let app_handle = app.handle().clone();
            let state_handle = app.state::<AppState>();

            let menu = tray::build_initial(&app_handle, &state_handle)?;
            TrayIconBuilder::with_id(tray::TRAY_ID)
                .menu(&menu)
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .expect("app icon configured in tauri.conf.json"),
                )
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    if let Some(index) = id.strip_prefix("primary:") {
                        handle_primary_pick(app, index);
                        return;
                    }
                    match id {
                        "settings" => routing::show_and_focus(app, "settings"),
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;

            // First launch: an http(s) URL in argv is routed immediately
            // (the settings window stays hidden, Linh stays resident in
            // the tray); otherwise the settings window is shown.
            let args: Vec<String> = std::env::args().collect();
            dispatch_launch(&app_handle, &state_handle, &args, process_start);

            // Drain any relaunches that raced in before AppState was
            // managed, in the order they arrived.
            for (received_at, args) in PENDING_LAUNCHES.lock().unwrap().drain() {
                dispatch_launch(&app_handle, &state_handle, &args, received_at);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_browsers,
            commands::get_config,
            commands::save_config,
            commands::get_platform_caps,
            commands::is_default_browser,
            commands::set_default_browser,
            commands::get_prompt_url,
            commands::prompt_choose,
            commands::prompt_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running linh");
}

fn handle_primary_pick(app: &AppHandle, index_str: &str) {
    let Ok(index) = index_str.parse::<usize>() else {
        return;
    };
    let state = app.state::<AppState>();
    let target = state.primary_targets.lock().unwrap().get(index).cloned();
    let Some(target) = target else {
        return;
    };

    let mut config = state.config.lock().unwrap().clone();
    config.primary = target;
    if let Err(err) = state.persist_config(config) {
        platform::linux::logging::log_line(&state.dirs, &format!("failed to save config: {err}"));
        return;
    }
    tray::sync_checked_state(&state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_before_ready_then_drain_preserves_order() {
        let mut queue = PendingLaunches::default();

        // Draining an empty queue (nothing enqueued yet, i.e. "ready
        // from the start") yields nothing.
        assert!(queue.drain().is_empty());

        // Two launches race in while AppState isn't managed yet.
        queue.enqueue(
            Instant::now(),
            vec!["linh".to_string(), "https://a.example".to_string()],
        );
        queue.enqueue(
            Instant::now(),
            vec!["linh".to_string(), "https://b.example".to_string()],
        );

        // Draining once AppState is ready returns both, oldest first.
        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(
            drained[0].1,
            vec!["linh".to_string(), "https://a.example".to_string()]
        );
        assert_eq!(
            drained[1].1,
            vec!["linh".to_string(), "https://b.example".to_string()]
        );

        // The queue is now empty again.
        assert!(queue.drain().is_empty());
    }
}
