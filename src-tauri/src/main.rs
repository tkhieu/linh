#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod platform;
mod routing;
mod state;
mod tray;

use tauri::tray::TrayIconBuilder;
use tauri::Manager;

use platform::linux::url_intake;
use state::AppState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch hands its argv to this already-running
            // instance via `args`. Route any http(s) URL in it; otherwise
            // just surface the settings window.
            let state = app.state::<AppState>();
            match url_intake::first_routable_url(&args) {
                Some(url) => routing::route_url(app, &state, &url),
                None => routing::show_and_focus(app, "settings"),
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
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
            match url_intake::first_routable_url(args.iter().skip(1)) {
                Some(url) => {
                    let state_handle = app.state::<AppState>();
                    routing::route_url(&app_handle, &state_handle, &url);
                }
                None => routing::show_and_focus(&app_handle, "settings"),
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

fn handle_primary_pick(app: &tauri::AppHandle, index_str: &str) {
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
