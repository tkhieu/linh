use std::sync::atomic::Ordering;
use std::time::Instant;

use linh_core::{AppTarget, Browser, Decision, LaunchCommand, PlatformCaps, Target};
use tauri::{AppHandle, Manager};
use url::Url;

use crate::platform::current::apps;
use crate::platform::current::dirs::Dirs;
use crate::platform::current::notify;
use crate::platform::launcher::{AsyncOutcomeHandler, LaunchOutcome};
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
    open_target_with(state, target, url, |cmd, on_outcome| {
        real_spawn(&state.dirs, cmd, on_outcome)
    })
}

/// What [`plan_app_open`] decided to do for a given `AppTarget` + URL —
/// kept separate from actually doing it (spawning a process is always
/// injected, never performed inline here) so the decision itself is a
/// pure function.
#[derive(Debug, Clone, PartialEq)]
enum AppOpenPlan {
    /// Launch the app with `cmd`; if spawning it fails, the caller must
    /// still fall through to `fallback`. `app_display_name` is the
    /// launched app's own name (e.g. "Microsoft Teams"), carried along
    /// so an `Unconfirmed` outcome can name the app that was actually
    /// launched, not `fallback`.
    Launch {
        cmd: LaunchCommand,
        fallback: Target,
        app_display_name: String,
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

    // Teams needs its web link rewritten to an `msteams://` deep link on
    // Windows and macOS, whose official clients only register as
    // handlers for that scheme, not for raw `https` URLs — Linux's
    // teams-for-linux accepts the original URL unchanged instead. Zoom
    // needs its web-join link rewritten to a `zoommtg://` deep link
    // everywhere, which isn't possible for every Zoom URL (see
    // `zoom_app_url`).
    let launch_url = match app_target.app.as_str() {
        "zoom" => linh_core::zoom_app_url(url),
        "teams" => {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                apps::teams_app_url(url)
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                Some(url.to_string())
            }
        }
        _ => None,
    };
    let Some(launch_url) = launch_url else {
        return AppOpenPlan::Fallback(fallback);
    };

    let Some(app) = find_app(&app_target.app) else {
        return AppOpenPlan::Fallback(fallback);
    };
    let app_display_name = app.display_name.clone();
    let dummy_target = Target {
        browser: app_target.app.clone(),
        profile: None,
        private: false,
    };
    match launcher::build_launch_command(&app, &dummy_target, &launch_url) {
        Ok(cmd) => AppOpenPlan::Launch {
            cmd,
            fallback,
            app_display_name,
        },
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
        |cmd, on_outcome| real_spawn(&state.dirs, cmd, on_outcome),
    )
}

fn open_app_target_with(
    state: &AppState,
    app_target: &AppTarget,
    url: &str,
    find_app: impl FnOnce(&str) -> Option<Browser>,
    spawn: impl Fn(&LaunchCommand, AsyncOutcomeHandler) -> Result<(), String>,
) -> Result<(), String> {
    let primary = state.config.lock().unwrap().primary.clone();
    match plan_app_open(app_target, url, find_app, &primary) {
        AppOpenPlan::Launch {
            cmd,
            fallback,
            app_display_name,
        } => {
            let on_outcome = app_launch_outcome_handler(
                state,
                fallback.clone(),
                app_display_name,
                url.to_string(),
            );
            if spawn(&cmd, on_outcome).is_ok() {
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
    spawn: impl Fn(&LaunchCommand, AsyncOutcomeHandler) -> Result<(), String>,
) -> Result<(), String> {
    let degraded = degrade_target(state.caps, target);
    let target = degraded.as_ref().unwrap_or(target);
    if degraded.is_some() {
        notify_profile_private_unsupported_once(state);
    }
    let browsers = state.browsers.lock().unwrap();
    let browser = browsers
        .iter()
        .find(|b| b.id == target.browser)
        .ok_or_else(|| format!("browser '{}' is not installed", target.browser))?;
    let cmd = launcher::build_launch_command(browser, target, url)?;
    drop(browsers);
    spawn(
        &cmd,
        direct_open_outcome_handler(state, target.clone(), url.to_string()),
    )
}

/// This is the single chokepoint every launch passes through —
/// `route_url`'s `Decision::Open`, a rule's `Target` fallback, and an
/// `AppTarget`'s browser fallback all end up calling [`open_target_with`]
/// — so stripping `profile`/`private` here, right before a `Browser` is
/// even looked up, is enough to guarantee `linh_core::decide`'s own
/// output (which never knows about platform capabilities) never reaches
/// a launch carrying a flag this platform can't deliver.
///
/// Returns `None` when `target` needed no change — including when
/// `caps.restrict_profile_private` is `false` — so callers can tell a
/// degradation actually happened (and so only show the user a notice
/// then) without re-deriving that themselves.
fn degrade_target(caps: PlatformCaps, target: &Target) -> Option<Target> {
    if !caps.restrict_profile_private {
        return None;
    }
    if target.profile.is_none() && !target.private {
        return None;
    }
    Some(Target {
        browser: target.browser.clone(),
        profile: None,
        private: false,
    })
}

/// Tells the user, once per run, that a profile or private-window target
/// degraded to the plain browser window because this build can't carry
/// either to a launch (see [`degrade_target`]) — a config full of rules
/// that each name one must not notify once per routed link.
fn notify_profile_private_unsupported_once(state: &AppState) {
    if state
        .profile_private_notice_shown
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    notify::notify_info(
        &state.dirs,
        "Linh: profile and private windows unavailable",
        "This Mac App Store build can't open a specific browser profile or \
         a private/incognito window in another app — opening it normally \
         instead.",
    );
}

/// Outcome handler for a direct (non-app) target open: there's no
/// further fallback target to try (a plain `Decision::Open` already is
/// the final destination), so a [`LaunchOutcome::DefiniteFailure`] is
/// just logged, and [`LaunchOutcome::Unconfirmed`] only notifies the
/// user — never retried, since `open` may still succeed on its own and
/// retrying too risks the link opening twice. Nothing extra on success:
/// the synchronous `receive->spawn` log line in [`route_url`] already
/// covers that case.
fn direct_open_outcome_handler(
    state: &AppState,
    target: Target,
    url: String,
) -> AsyncOutcomeHandler {
    let dirs = state.dirs.clone();
    Box::new(move |outcome| match outcome {
        LaunchOutcome::Succeeded => {}
        LaunchOutcome::DefiniteFailure => {
            logging::log_line(
                &dirs,
                &format!("{url}: confirmed failed to open in {target:?}"),
            );
        }
        LaunchOutcome::Unconfirmed => notify_could_not_confirm(&dirs, &url, &target.browser),
    })
}

/// Outcome handler for an app-target's launch attempt: a
/// [`LaunchOutcome::DefiniteFailure`] runs `fallback` exactly once (see
/// [`run_watched_fallback`], which never chains a further fallback of
/// its own); [`LaunchOutcome::Unconfirmed`] only notifies, naming the
/// app that was launched (`app_display_name`, e.g. "Microsoft Teams"),
/// never `fallback` — nothing has actually tried `fallback` yet, so
/// naming it there would blame the wrong target. `open` may still
/// succeed on its own, and falling back too risks the link opening
/// twice. Nothing extra on success.
///
/// Snapshots `state.dirs`/`state.browsers` now rather than reading the
/// live `AppState` when this eventually runs (possibly seconds later,
/// from a worker thread): `AppState::browsers` is a plain
/// `Mutex<Vec<Browser>>`, not behind an `Arc` this closure could hold
/// its own clone of and a `tauri::AppHandle` isn't available this deep
/// into `routing` without threading one through every caller (`route_url`
/// has one; the plain `open_target`/`open_app_target` entry points used
/// directly by `commands::prompt_choose` and tests do not) — making the
/// live state reachable would need that restructuring. A browser list
/// changing in the few seconds before a possible confirmed failure is
/// rare enough to accept against that cost.
fn app_launch_outcome_handler(
    state: &AppState,
    fallback: Target,
    app_display_name: String,
    url: String,
) -> AsyncOutcomeHandler {
    let dirs = state.dirs.clone();
    let browsers = state.browsers.lock().unwrap().clone();
    Box::new(move |outcome| match outcome {
        LaunchOutcome::Succeeded => {}
        LaunchOutcome::Unconfirmed => notify_could_not_confirm(&dirs, &url, &app_display_name),
        LaunchOutcome::DefiniteFailure => run_watched_fallback(dirs, browsers, fallback, url),
    })
}

/// Runs `fallback` for `url` exactly once, watching its own outcome too
/// (rather than assuming success as soon as it's spawned) — but never
/// chains a further fallback if it fails or can't be confirmed either:
/// that's reported with the same [`notify_could_not_confirm`] path and
/// logged, not retried, to keep this bounded to a single hop.
fn run_watched_fallback(dirs: Dirs, browsers: Vec<Browser>, fallback: Target, url: String) {
    let dirs_for_outcome = dirs.clone();
    let fallback_for_outcome = fallback.clone();
    let url_for_outcome = url.clone();
    let on_fallback_outcome: AsyncOutcomeHandler = Box::new(move |outcome| match outcome {
        LaunchOutcome::Succeeded => logging::log_line(
            &dirs_for_outcome,
            &format!("{url_for_outcome}: fallback {fallback_for_outcome:?} confirmed opened"),
        ),
        LaunchOutcome::DefiniteFailure | LaunchOutcome::Unconfirmed => {
            logging::log_line(
                &dirs_for_outcome,
                &format!(
                    "{url_for_outcome}: fallback {fallback_for_outcome:?} failed ({outcome:?}); \
                     not chaining a further fallback"
                ),
            );
            notify_could_not_confirm(
                &dirs_for_outcome,
                &url_for_outcome,
                &fallback_for_outcome.browser,
            );
        }
    });

    let result = browsers
        .iter()
        .find(|b| b.id == fallback.browser)
        .ok_or_else(|| format!("browser '{}' is not installed", fallback.browser))
        .and_then(|browser| launcher::build_launch_command(browser, &fallback, &url))
        .and_then(|cmd| real_spawn(&dirs, &cmd, on_fallback_outcome));
    if let Err(err) = result {
        logging::log_line(
            &dirs,
            &format!("{url}: fallback {fallback:?} could not even start: {err}"),
        );
        notify_could_not_confirm(&dirs, &url, &fallback.browser);
    }
}

/// Tells the user, via the platform's notification path, that Linh
/// couldn't confirm `url` actually opened in `target_name` — shown with
/// just the host (not the full URL, which may be long or carry
/// sensitive query parameters); the log line keeps the full URL.
fn notify_could_not_confirm(dirs: &Dirs, url: &str, target_name: &str) {
    let host = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string());
    notify::notify_error(
        dirs,
        "Linh: link may not have opened",
        &format!("Could not confirm opening {host} in {target_name}."),
    );
    logging::log_line(
        dirs,
        &format!("{url}: could not confirm opening in {target_name}"),
    );
}

/// The real, process-spawning implementation of the `spawn` parameter
/// threaded through [`open_target_with`]/[`open_app_target_with`]; tests
/// inject a fake instead so they never launch a real process.
fn real_spawn(
    dirs: &Dirs,
    cmd: &LaunchCommand,
    on_outcome: AsyncOutcomeHandler,
) -> Result<(), String> {
    launcher::spawn_detached(dirs, cmd, on_outcome).map_err(|e| e.to_string())
}

pub fn show_and_focus(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// `degrade_target` is pure platform-capability logic with no OS-specific
// I/O, so (unlike the fixtures below) these run on every host, including
// this repo's Linux CI — the only place `Dist::Mas`'s actual launch path
// ever gets a unit test at all, since the macOS-only code that builds
// its `caps.restrict_profile_private = true` is itself untestable off a
// real Mac (see `platform::macos::command`/`workspace`'s own doc
// comments).
#[cfg(test)]
mod degrade_target_tests {
    use super::*;

    fn target(browser: &str, profile: Option<&str>, private: bool) -> Target {
        Target {
            browser: browser.to_string(),
            profile: profile.map(str::to_string),
            private,
        }
    }

    fn caps(restrict_profile_private: bool) -> PlatformCaps {
        PlatformCaps {
            source_app_rules: false,
            restrict_profile_private,
        }
    }

    #[test]
    fn unrestricted_platform_never_degrades_anything() {
        assert_eq!(
            degrade_target(caps(false), &target("chrome", Some("Work"), true)),
            None
        );
        assert_eq!(
            degrade_target(caps(false), &target("chrome", None, false)),
            None
        );
    }

    #[test]
    fn restricted_platform_leaves_a_plain_target_alone() {
        assert_eq!(
            degrade_target(caps(true), &target("chrome", None, false)),
            None
        );
    }

    #[test]
    fn restricted_platform_strips_profile() {
        assert_eq!(
            degrade_target(caps(true), &target("chrome", Some("Work"), false)),
            Some(target("chrome", None, false))
        );
    }

    #[test]
    fn restricted_platform_strips_private() {
        assert_eq!(
            degrade_target(caps(true), &target("chrome", None, true)),
            Some(target("chrome", None, false))
        );
    }

    #[test]
    fn restricted_platform_strips_both_at_once_and_keeps_the_browser_id() {
        assert_eq!(
            degrade_target(caps(true), &target("com.google.chrome", Some("Work"), true)),
            Some(target("com.google.chrome", None, false))
        );
    }
}

// These fixtures are Desktop Entry commands (raw `Exec=` strings,
// `cmd.program` equal to the launched browser's own binary, a spawn
// failure meaning the binary itself couldn't exec) — Windows registry
// commands are exercised in platform::windows::command tests, and macOS
// `open`-wrapped commands (`cmd.program` is always `/usr/bin/open`,
// `launcher::watch_and_report`'s confirmed-failure handling standing in
// for a spawn failure) in platform::launcher's and
// platform::macos::command's own tests.
#[cfg(all(test, target_os = "linux"))]
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

    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: Vec::new(),
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
            AppOpenPlan::Launch {
                cmd,
                fallback,
                app_display_name,
            } => {
                assert_eq!(cmd.program, "/usr/bin/teams-for-linux");
                assert_eq!(
                    cmd.args,
                    vec!["https://teams.microsoft.com/l/meetup-join/abc".to_string()]
                );
                assert_eq!(fallback, target("google-chrome"));
                assert_eq!(app_display_name, "teams");
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
            |cmd, _on_outcome| {
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
            |cmd, _on_outcome| {
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
    fn unconfirmed_app_launch_names_the_app_not_the_fallback() {
        let root = temp_root("unconfirmed-app-name");
        let state = test_state(
            &root,
            vec![native_browser("google-chrome", "/usr/bin/x %u")],
        );
        let app_target = app_target("teams", Some(target("google-chrome")));
        let app = Browser {
            id: "teams".to_string(),
            display_name: "Teams for Linux".to_string(),
            family: BrowserFamily::Other,
            install: InstallKind::Native {
                exec: "/usr/bin/teams-for-linux %u".to_string(),
            },
            profiles: vec![],
            supports_private: false,
            icon: None,
            desktop_file: None,
        };
        let original_url = "https://teams.microsoft.com/l/meetup-join/abc";
        let captured: RefCell<Option<AsyncOutcomeHandler>> = RefCell::new(None);

        let result = open_app_target_with(
            &state,
            &app_target,
            original_url,
            |_| Some(app.clone()),
            |cmd, on_outcome| {
                assert_eq!(cmd.program, "/usr/bin/teams-for-linux");
                *captured.borrow_mut() = Some(on_outcome);
                Ok(())
            },
        );
        assert!(result.is_ok());

        let on_outcome = captured
            .into_inner()
            .expect("the app spawn must hand the outcome handler to the caller");
        on_outcome(LaunchOutcome::Unconfirmed);

        let log = std::fs::read_to_string(test_dirs(&root).log_path()).unwrap_or_default();
        assert!(
            log.contains("could not confirm opening in Teams for Linux"),
            "must name the launched app, not the fallback browser; log: {log}"
        );
        assert!(
            !log.contains("could not confirm opening in google-chrome"),
            "must never name the fallback for the original launch's own outcome; log: {log}"
        );
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
            |cmd, _on_outcome| {
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
