#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod allocator;
mod commands;
mod mem_profile;
mod platform;
mod prompt_state;
mod routing;
mod settings_window;
mod state;
mod tray;
mod webkit_tune;

use std::sync::Mutex;
use std::time::Instant;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use platform::url_intake;
use state::AppState;

/// Registry for launches (argv + the moment each was received) that race
/// in via the single-instance callback before `setup` has `app.manage`d
/// `AppState` yet. `Some(queue)` means "not ready": any launch must be
/// queued rather than routed, since `AppState` may not be managed. `None`
/// means "ready": `app.manage` has already run, so a launch can be
/// routed directly via `try_state` without risking a panic.
///
/// The check ("are we ready?") and the enqueue must happen as one atomic
/// step under the lock — checking `try_state()` and *then* enqueuing
/// (two separate lock-free steps) leaves a window where `setup` marks
/// the registry ready and drains it in between, permanently losing the
/// URL. [`enqueue_if_pending`] and [`mark_ready`] below only ever run
/// while holding `PENDING_LAUNCHES`'s lock, closing that window.
static PENDING_LAUNCHES: Mutex<PendingRegistry> = Mutex::new(Some(Vec::new()));

/// `None` once `setup` has `app.manage`d `AppState`; until then,
/// `Some(queue)` of each queued launch's argv and receive timestamp.
type PendingRegistry = Option<Vec<(Instant, Vec<String>)>>;

/// If `registry` is still pending, queues `(received_at, args)` and
/// returns `None`. If `registry` is already marked ready, hands `args`
/// back unchanged so the caller can route it directly instead of queuing
/// something that will now never be drained.
fn enqueue_if_pending(
    registry: &mut PendingRegistry,
    received_at: Instant,
    args: Vec<String>,
) -> Option<Vec<String>> {
    match registry {
        Some(queue) => {
            queue.push((received_at, args));
            None
        }
        None => Some(args),
    }
}

/// Takes the queued launches out of `registry` for dispatching, leaving
/// it `Some(vec![])` — still *not* ready. Call [`mark_ready`] only after
/// every launch this returns has actually been dispatched, without ever
/// releasing the registry's lock in between: a newer single-instance
/// callback needs that same lock to enqueue or to check readiness, so
/// holding it across the dispatch loop makes that callback wait until
/// every older queued URL has been dispatched first — preserving
/// arrival order, so the most recently dispatched one is left as the
/// prompt's pending URL ("latest wins").
fn drain_pending(registry: &mut PendingRegistry) -> Vec<(Instant, Vec<String>)> {
    registry.as_mut().map(std::mem::take).unwrap_or_default()
}

/// Marks `registry` ready (`None`). Idempotent.
fn mark_ready(registry: &mut PendingRegistry) {
    *registry = None;
}

/// Routes `args`' first `http`/`https` URL (logged, timed from
/// `received_at`), or shows the settings window if there is none —
/// creating it on demand (see [`settings_window::show_or_create`]) the
/// first time it's needed.
fn dispatch_launch(app: &AppHandle, state: &AppState, args: &[String], received_at: Instant) {
    match url_intake::first_routable_url(args) {
        Some(url) => routing::route_url(app, state, &url, received_at),
        None => settings_window::show_or_create(app, &state.dirs),
    }
}

fn main() {
    allocator::cap_arenas();
    // Must run before the builder below builds its first window (and so
    // its first `WebContext`/`WebsiteDataManager`) — see webkit_tune's
    // doc comment for why this can't instead happen from `setup`.
    webkit_tune::configure_before_tauri();
    // Captured before anything else so first-launch latency logging
    // honestly includes browser scanning and config load, not just the
    // final routing decision.
    let process_start = Instant::now();

    tauri::Builder::default()
        .on_page_load(move |webview, payload| {
            if let Some(state) = webview.try_state::<AppState>() {
                mem_profile::log_window_ready_if_relevant(
                    &state.dirs,
                    process_start,
                    webview.label(),
                    payload.event(),
                );
            }
        })
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // The moment this already-running instance was actually
            // notified of the second launch's argv.
            let received_at = Instant::now();
            let not_queued = {
                let mut registry = PENDING_LAUNCHES.lock().unwrap();
                enqueue_if_pending(&mut registry, received_at, args)
            };
            // `not_queued` is `Some(args)` only when the registry was
            // already marked ready, which `setup` only does *after*
            // `app.manage(AppState)` — so `try_state` is guaranteed to
            // succeed here; the `if let` is defensive, not a silent drop.
            if let Some(args) = not_queued {
                if let Some(state) = app.try_state::<AppState>() {
                    dispatch_launch(app, &state, &args, received_at);
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            let state = AppState::init();
            app.manage(state);
            let app_handle = app.handle().clone();
            let state_handle = app.state::<AppState>();

            // macOS delivers an http(s) link as an Apple Event
            // (`kAEGetURL`), not argv: Tauri surfaces it as
            // `RunEvent::Opened`, which `tauri-plugin-deep-link` forwards
            // as a `deep-link://new-url` event. Feeding that into the
            // very same `PENDING_LAUNCHES` queue the single-instance
            // callback above uses — rather than also polling
            // `deep_link().get_current()` for the launch that opened
            // Linh in the first place — keeps there being exactly one
            // dispatch per URL: once this listener is registered (here,
            // before anything below that could block), every `Opened`
            // event is either queued (not ready yet) or dispatched
            // immediately (already ready), the same as a second launch.
            #[cfg(target_os = "macos")]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_for_listener = app_handle.clone();
                app_handle.deep_link().on_open_url(move |event| {
                    let received_at = Instant::now();
                    let args: Vec<String> = event
                        .urls()
                        .into_iter()
                        .map(|url| url.to_string())
                        .collect();
                    let not_queued = {
                        let mut registry = PENDING_LAUNCHES.lock().unwrap();
                        enqueue_if_pending(&mut registry, received_at, args)
                    };
                    if let Some(args) = not_queued {
                        if let Some(state) = app_for_listener.try_state::<AppState>() {
                            dispatch_launch(&app_for_listener, &state, &args, received_at);
                        }
                    }
                });
            }

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
                        "settings" => {
                            let state = app.state::<AppState>();
                            settings_window::show_or_create(app, &state.dirs);
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;

            #[cfg(target_os = "linux")]
            if let Some(prompt_window) = app_handle.get_webview_window("prompt") {
                let dirs_for_err = state_handle.dirs.clone();
                let scheduled = prompt_window.with_webview(|webview| {
                    webkit_tune::tune_webview(webview.inner());
                });
                if let Err(err) = scheduled {
                    platform::logging::log_line(
                        &dirs_for_err,
                        &format!("webkit_tune: with_webview failed: {err}"),
                    );
                }
            }

            mem_profile::spawn_if_enabled(&app_handle, state_handle.dirs.clone());

            // First launch: an http(s) URL in argv is routed immediately
            // (no window shown, Linh stays resident in the tray);
            // otherwise the settings window is created (first time) or
            // shown.
            let args: Vec<String> = std::env::args().collect();
            dispatch_launch(&app_handle, &state_handle, &args, process_start);

            // Drain and dispatch whatever raced in before this point,
            // oldest first, then mark the registry ready — all while
            // holding its lock continuously, so a newer single-instance
            // callback blocks (on the same lock) until these are fully
            // dispatched rather than possibly racing ahead of them.
            {
                let mut registry = PENDING_LAUNCHES.lock().unwrap();
                for (received_at, args) in drain_pending(&mut registry) {
                    dispatch_launch(&app_handle, &state_handle, &args, received_at);
                }
                mark_ready(&mut registry);
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
            commands::list_presets,
            commands::get_icon,
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

    if let Err(err) = state.update_config(|cfg| cfg.primary = target) {
        platform::logging::log_line(&state.dirs, &format!("failed to save config: {err}"));
        return;
    }
    tray::sync_checked_state(&state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_while_pending_is_queued_not_handed_back() {
        let mut registry: PendingRegistry = Some(Vec::new());

        let result = enqueue_if_pending(&mut registry, Instant::now(), vec!["a".to_string()]);

        assert!(result.is_none(), "must be queued, not handed back");
        assert_eq!(registry.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn drain_pending_returns_queued_items_in_order_and_stays_not_ready() {
        let mut registry: PendingRegistry = Some(Vec::new());
        enqueue_if_pending(&mut registry, Instant::now(), vec!["first".to_string()]);
        enqueue_if_pending(&mut registry, Instant::now(), vec!["second".to_string()]);

        let drained = drain_pending(&mut registry);

        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].1, vec!["first".to_string()]);
        assert_eq!(drained[1].1, vec!["second".to_string()]);
        // drain_pending alone must NOT mark the registry ready: a
        // caller must still call mark_ready only after dispatching.
        assert!(
            registry.is_some(),
            "registry must still be pending (Some) right after draining, before mark_ready"
        );
        assert!(registry.unwrap().is_empty());
    }

    #[test]
    fn drain_pending_on_an_empty_pending_registry_returns_nothing() {
        let mut registry: PendingRegistry = Some(Vec::new());
        assert!(drain_pending(&mut registry).is_empty());
        assert!(registry.is_some());
    }

    #[test]
    fn mark_ready_after_drain_marks_the_registry_ready() {
        let mut registry: PendingRegistry = Some(Vec::new());
        enqueue_if_pending(&mut registry, Instant::now(), vec!["first".to_string()]);

        let drained = drain_pending(&mut registry);
        assert_eq!(drained.len(), 1);
        mark_ready(&mut registry);

        assert!(registry.is_none(), "registry must be marked ready (None)");
    }

    #[test]
    fn enqueue_after_ready_is_refused_so_caller_routes_directly() {
        let mut registry: PendingRegistry = Some(Vec::new());
        mark_ready(&mut registry); // registry is now ready (None).

        let args = vec!["late".to_string()];
        let handed_back = enqueue_if_pending(&mut registry, Instant::now(), args.clone());

        assert_eq!(
            handed_back,
            Some(args),
            "must hand args back for the caller to route, never silently drop them"
        );
    }

    #[test]
    fn mark_ready_is_idempotent_once_already_ready() {
        let mut registry: PendingRegistry = None;
        mark_ready(&mut registry);
        assert!(registry.is_none());
    }
}
