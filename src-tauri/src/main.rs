#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch hands its URL to this already-running
            // instance; wiring that URL through to routing is phase 2's
            // job (src-tauri backend). For now, just surface the window.
            if let Some(window) = app.get_webview_window("settings") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            let open_settings =
                MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_settings, &quit])?;

            TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().cloned().expect("app icon configured in tauri.conf.json"))
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "settings" => {
                        if let Some(window) = app.get_webview_window("settings") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

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
