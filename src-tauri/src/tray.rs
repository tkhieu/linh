use linh_core::Target;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, Submenu};
use tauri::{AppHandle, Wry};

use crate::state::AppState;

pub const TRAY_ID: &str = "main";

struct Entry {
    label: String,
    target: Target,
}

fn primary_entries(state: &AppState) -> Vec<Entry> {
    let browsers = state.browsers.lock().unwrap();
    let mut entries = Vec::new();
    for browser in browsers.iter() {
        entries.push(Entry {
            label: browser.display_name.clone(),
            target: Target {
                browser: browser.id.clone(),
                profile: None,
                private: false,
            },
        });
        for profile in &browser.profiles {
            entries.push(Entry {
                label: format!("{} — {}", browser.display_name, profile.display_name),
                target: Target {
                    browser: browser.id.clone(),
                    profile: Some(profile.dir.clone()),
                    private: false,
                },
            });
        }
        if browser.supports_private {
            entries.push(Entry {
                label: format!("{} (Private)", browser.display_name),
                target: Target {
                    browser: browser.id.clone(),
                    profile: None,
                    private: true,
                },
            });
        }
    }
    entries
}

/// Builds the tray menu ("Primary Browser" submenu, "Settings…", "Quit")
/// once, and stores the built `CheckMenuItem`s (and the `Target` each one
/// corresponds to, by the same index) in `state` so that:
/// - the click handler can map a `primary:<index>` menu id back to a
///   `Target`;
/// - [`sync_checked_state`] can later update which item is checked
///   in place, without replacing the menu object (tauri's tray menu
///   cannot be swapped out on Linux once set, only mutated).
pub fn build_initial(app: &AppHandle<Wry>, state: &AppState) -> tauri::Result<Menu<Wry>> {
    let entries = primary_entries(state);
    let current_primary = state.config.lock().unwrap().primary.clone();

    let mut items: Vec<CheckMenuItem<Wry>> = Vec::with_capacity(entries.len());
    let mut targets: Vec<Target> = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.into_iter().enumerate() {
        let checked = entry.target == current_primary;
        let item = CheckMenuItem::with_id(
            app,
            format!("primary:{idx}"),
            &entry.label,
            true,
            checked,
            None::<&str>,
        )?;
        items.push(item);
        targets.push(entry.target);
    }

    let item_refs: Vec<&dyn IsMenuItem<Wry>> =
        items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let primary_submenu = Submenu::with_items(app, "Primary Browser", true, &item_refs)?;

    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&primary_submenu, &settings_item, &quit_item])?;

    *state.primary_targets.lock().unwrap() = targets;
    *state.primary_items.lock().unwrap() = items;

    Ok(menu)
}

/// Re-checks the tray's "Primary Browser" items in place to match the
/// current config, e.g. after a tray click or `save_config`.
pub fn sync_checked_state(state: &AppState) {
    let current_primary = state.config.lock().unwrap().primary.clone();
    let targets = state.primary_targets.lock().unwrap();
    let items = state.primary_items.lock().unwrap();
    for (target, item) in targets.iter().zip(items.iter()) {
        let _ = item.set_checked(*target == current_primary);
    }
}
