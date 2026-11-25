use std::time::Instant;

use linh_core::{AppTarget, Browser, Decision, LaunchCommand, Target};
use tauri::{AppHandle, Manager};
use url::Url;

use crate::platform::current::apps;
use crate::platform::{launcher, logging, url_intake};
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
    open_target_with(state, target, url, real_spawn)
}

/// What [`plan_app_open`] decided to do for a given `AppTarget` + URL —
/// kept separate from actually doing it (spawning a process is always
/// injected, never performed inline here) so the decision itself is a
/// pure function.
#[derive(Debug, Clone, PartialEq)]
enum AppOpenPlan {
    /// Launch the app with `cmd`; if spawning it fails, the caller must
    /// still fall through to `fallback`.
    Launch {
        cmd: LaunchCommand,
        fallback: Target,
    },
    /// The app isn't installed, or `url` can't be converted for it:
    /// open `fallback` directly, with the original (unconverted) URL.
    Fallback(Target),
}

/// Pure routing decision for an `AppTarget` rule: whether to launch the
/// app, and to which target to fall back if not (either
/// `app_target.fallback`, or `primary` when that's `None`). Takes
/// `find_app` as a parameter rather than calling `apps::find_app`
/// directly so this never touches the filesystem itself — the real
/// detection scan is injected by [`open_app_target`], a fixed lookup by
/// tests.
fn plan_app_open(
    app_target: &AppTarget,
    url: &str,
    find_app: impl FnOnce(&str) -> Option<Browser>,
    primary: &Target,
) -> AppOpenPlan {
    let fallback = app_target
        .fallback
        .clone()
        .unwrap_or_else(|| primary.clone());

    // Teams always passes the original URL through unchanged; Zoom
    // needs its web-join link rewritten to a `zoommtg://` deep link,
    // which isn't possible for every Zoom URL (see `zoom_app_url`).
    let launch_url = match app_target.app.as_str() {
        "zoom" => linh_core::zoom_app_url(url),
        "teams" => Some(url.to_string()),
        _ => None,
    };
    let Some(launch_url) = launch_url else {
        return AppOpenPlan::Fallback(fallback);
    };

    let Some(app) = find_app(&app_target.app) else {
        return AppOpenPlan::Fallback(fallback);
    };
    let dummy_target = Target {
        browser: app_target.app.clone(),
        profile: None,
        private: false,
    };
    match linh_core::build_launch_command(&app, &dummy_target, &launch_url) {
        Ok(cmd) => AppOpenPlan::Launch { cmd, fallback },
        Err(_) => AppOpenPlan::Fallback(fallback),
    }
}

/// Resolves `app_target` for `url` and carries it out: launches the app
/// (never through a shell) when it's installed and `url` can be
/// converted for it; otherwise — app not installed, URL not
/// convertible, or the spawn itself fails — opens `app_target.fallback`,
/// or `Config::primary` when there is no fallback, with the original
/// (unconverted) `url`. Returns `Err` only if that fallback open also
/// fails.
pub fn open_app_target(state: &AppState, app_target: &AppTarget, url: &str) -> Result<(), String> {
    open_app_target_with(
        state,
        app_target,
        url,
        |app_id| apps::find_app(&state.dirs, app_id),
        real_spawn,
    )
}

fn open_app_target_with(
    state: &AppState,
    app_target: &AppTarget,
    url: &str,
    find_app: impl FnOnce(&str) -> Option<Browser>,
    spawn: impl Fn(&LaunchCommand) -> Result<(), String>,
) -> Result<(), String> {
    let primary = state.config.lock().unwrap().primary.clone();
    match plan_app_open(app_target, url, find_app, &primary) {
        AppOpenPlan::Launch { cmd, fallback } => {
            if spawn(&cmd).is_ok() {
                Ok(())
            } else {
                open_target_with(state, &fallback, url, spawn)
            }
        }
        AppOpenPlan::Fallback(fallback) => open_target_with(state, &fallback, url, spawn),
    }
}

fn open_target_with(
    state: &AppState,
    target: &Target,
    url: &str,
    spawn: impl Fn(&LaunchCommand) -> Result<(), String>,
) -> Result<(), String> {
    let browsers = state.browsers.lock().unwrap();
    let browser = browsers
        .iter()
        .find(|b| b.id == target.browser)
        .ok_or_else(|| format!("browser '{}' is not installed", target.browser))?;
    let cmd = linh_core::build_launch_command(browser, target, url).map_err(|e| e.to_string())?;
    drop(browsers);
    spawn(&cmd)
}

/// The real, process-spawning implementation of the `spawn` parameter
/// threaded through [`open_target_with`]/[`open_app_target_with`]; tests
/// inject a fake instead so they never launch a real process.
fn real_spawn(cmd: &LaunchCommand) -> Result<(), String> {
    launcher::spawn_detached(cmd).map_err(|e| e.to_string())
}

pub fn show_and_focus(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linh_core::{BrowserFamily, Config, InstallKind};
    use std::cell::RefCell;
    use std::path::PathBuf;

    use crate::platform::current::dirs::Dirs;

    fn target(browser: &str) -> Target {
        Target {
            browser: browser.to_string(),
            profile: None,
            private: false,
        }
    }

    fn app_target(app: &str, fallback: Option<Target>) -> AppTarget {
        AppTarget {
            app: app.to_string(),
            fallback,
        }
    }

    fn native_browser(id: &str, exec: &str) -> Browser {
        Browser {
            id: id.to_string(),
            display_name: id.to_string(),
            family: BrowserFamily::Other,
            install: InstallKind::Native {
                exec: exec.to_string(),
            },
            profiles: Vec::new(),
            supports_private: false,
            icon: None,
            desktop_file: None,
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-routing-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(target_os = "linux")]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: Vec::new(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
        }
    }

    fn test_state(root: &std::path::Path, browsers: Vec<Browser>) -> AppState {
        AppState::for_test(test_dirs(root), Config::default(), browsers)
    }

    // --- `plan_app_open`: pure decision, no filesystem/process I/O ---

    #[test]
    fn app_present_and_convertible_launches_with_app_argv() {
        let app_target = app_target("teams", Some(target("google-chrome")));
        let app = native_browser("teams", "/usr/bin/teams-for-linux %u");

        let plan = plan_app_open(
            &app_target,
            "https://teams.microsoft.com/l/meetup-join/abc",
            |app_id| {
                assert_eq!(app_id, "teams");
                Some(app.clone())
            },
            &target("firefox"),
        );

        match plan {
            AppOpenPlan::Launch { cmd, fallback } => {
                assert_eq!(cmd.program, "/usr/bin/teams-for-linux");
                assert_eq!(
                    cmd.args,
                    vec!["https://teams.microsoft.com/l/meetup-join/abc".to_string()]
                );
                assert_eq!(fallback, target("google-chrome"));
            }
            other => panic!("expected Launch, got {other:?}"),
        }
    }

    #[test]
    fn zoom_convertible_url_launches_with_rewritten_argv() {
        let app_target = app_target("zoom", None);
        let app = native_browser("zoom", "/usr/bin/zoom %u");

        let plan = plan_app_open(
            &app_target,
            "https://us02web.zoom.us/j/123456789?pwd=abc",
            |_| Some(app.clone()),
            &target("firefox"),
        );

        match plan {
            AppOpenPlan::Launch { cmd, .. } => {
                assert_eq!(
                    cmd.args,
                    vec![
                        "zoommtg://us02web.zoom.us/join?action=join&confno=123456789&pwd=abc"
                            .to_string()
                    ]
                );
            }
            other => panic!("expected Launch, got {other:?}"),
        }
    }

    #[test]
    fn app_absent_falls_back_without_even_checking_url_convertibility_result() {
        let app_target = app_target("teams", Some(target("google-chrome")));

        let plan = plan_app_open(
            &app_target,
            "https://teams.microsoft.com/l/meetup-join/abc",
            |_| None,
            &target("firefox"),
        );

        assert_eq!(
            plan,
            AppOpenPlan::Fallback(target("google-chrome")),
            "app not installed must fall back, not panic or launch"
        );
    }

    #[test]
    fn zoom_url_not_convertible_falls_back_even_when_app_is_installed() {
        let app_target = app_target("zoom", Some(target("google-chrome")));
        let app = native_browser("zoom", "/usr/bin/zoom %u");

        let plan = plan_app_open(
            &app_target,
            "https://zoom.us/pricing",
            |_| Some(app.clone()),
            &target("firefox"),
        );

        assert_eq!(plan, AppOpenPlan::Fallback(target("google-chrome")));
    }

    #[test]
    fn no_fallback_configured_uses_primary() {
        let app_target = app_target("teams", None);

        let plan = plan_app_open(
            &app_target,
            "https://teams.microsoft.com/l/meetup-join/abc",
            |_| None,
            &target("firefox"),
        );

        assert_eq!(plan, AppOpenPlan::Fallback(target("firefox")));
    }

    // --- `open_app_target_with`: the composed, spawn-injected flow ---

    #[test]
    fn app_absent_fallback_gets_the_original_https_url() {
        let root = temp_root("absent-original-url");
        let state = test_state(
            &root,
            vec![native_browser("google-chrome", "/usr/bin/x %u")],
        );
        let app_target = app_target("zoom", Some(target("google-chrome")));
        let original_url = "https://us02web.zoom.us/j/123456789?pwd=abc";
        let seen: RefCell<Vec<LaunchCommand>> = RefCell::new(Vec::new());

        let result = open_app_target_with(
            &state,
            &app_target,
            original_url,
            |_| None, // app not installed
            |cmd| {
                seen.borrow_mut().push(cmd.clone());
                Ok(())
            },
        );

        assert!(result.is_ok());
        let calls = seen.into_inner();
        assert_eq!(calls.len(), 1, "only the fallback browser should spawn");
        assert_eq!(
            calls[0].args.last().unwrap(),
            original_url,
            "fallback must receive the original URL, never a Zoom-rewritten one"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn app_spawn_error_falls_back_and_fallback_gets_the_original_url() {
        let root = temp_root("spawn-error");
        let state = test_state(
            &root,
            vec![native_browser("google-chrome", "/usr/bin/x %u")],
        );
        let app_target = app_target("teams", Some(target("google-chrome")));
        let app = native_browser("teams", "/usr/bin/teams-for-linux %u");
        let original_url = "https://teams.microsoft.com/l/meetup-join/abc";
        let seen: RefCell<Vec<LaunchCommand>> = RefCell::new(Vec::new());

        let result = open_app_target_with(
            &state,
            &app_target,
            original_url,
            |_| Some(app.clone()),
            |cmd| {
                let is_app_cmd = cmd.program == "/usr/bin/teams-for-linux";
                seen.borrow_mut().push(cmd.clone());
                if is_app_cmd {
                    Err("spawn failed".to_string())
                } else {
                    Ok(())
                }
            },
        );

        assert!(
            result.is_ok(),
            "the fallback spawn succeeded, so the overall result must be Ok"
        );
        let calls = seen.into_inner();
        assert_eq!(calls.len(), 2, "the app launch attempt, then the fallback");
        assert_eq!(calls[0].program, "/usr/bin/teams-for-linux");
        assert_eq!(calls[1].program, "/usr/bin/x");
        assert_eq!(calls[1].args.last().unwrap(), original_url);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_fallback_and_app_absent_opens_primary() {
        let root = temp_root("primary");
        let config = Config {
            primary: target("firefox"),
            ..Config::default()
        };
        let state = AppState::for_test(
            test_dirs(&root),
            config,
            vec![native_browser("firefox", "/usr/bin/firefox %u")],
        );
        let app_target = app_target("teams", None);
        let seen: RefCell<Vec<LaunchCommand>> = RefCell::new(Vec::new());

        let result = open_app_target_with(
            &state,
            &app_target,
            "https://teams.microsoft.com/l/meetup-join/abc",
            |_| None,
            |cmd| {
                seen.borrow_mut().push(cmd.clone());
                Ok(())
            },
        );

        assert!(result.is_ok());
        let calls = seen.into_inner();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "/usr/bin/firefox");
        std::fs::remove_dir_all(&root).ok();
    }
}
