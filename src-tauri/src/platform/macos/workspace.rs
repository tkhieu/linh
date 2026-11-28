//! Sandboxed-safe launching and default-handler registration for the Mac
//! App Store build (`LINH_DIST=mas`), via `NSWorkspace` (AppKit) instead
//! of exec'ing `/usr/bin/open` or calling the LaunchServices C functions
//! that *write* state.
//!
//! The App Sandbox gives a signed, sandboxed app no exec rights over
//! arbitrary system binaries — `open`(1), `osascript`(1), `sips`(1) are
//! all out — so `command.rs`'s `open`-wrapped `LaunchCommand`s (and this
//! module's own callers in `platform::launcher`) only ever reach this
//! module once `updates::current_dist()` is [`crate::updates::Dist::Mas`].
//! `LSSetDefaultHandlerForURLScheme` is likewise not usable from the
//! sandbox (see `default_browser.rs`'s MAS path), unlike the read-only
//! `LSCopy*` queries `launch_services.rs` still uses unconditionally.
//!
//! `NSWorkspace.open(urls:withApplicationAt:configuration:)` never
//! forwards launch arguments to the opened app when sandboxed — by
//! sandbox design, not merely this module's own choice — which is
//! exactly why this module never offers a way to pass any:
//! [`open_with_app`] always opens with a fresh, argument-less
//! `NSWorkspaceOpenConfiguration`, and `routing::degrade_target` strips
//! any `profile`/`private` off a `Target` before it ever reaches
//! `build_launch_command`, so a caller here is never even tempted to
//! try.
//!
//! Every AppKit call in this module goes through its asynchronous,
//! completion-handler form, never a synchronous one — `route_url` (and
//! so, transitively, every function here) can run inline during
//! `main.rs`'s `setup` closure, on Tauri's main thread, for a URL
//! received at cold start; a synchronous round-trip to LaunchServices/
//! the Dock there would block the whole app's startup on it.

use objc2::rc::Retained;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSError, NSString, NSURL};
use std::sync::Mutex;

use crate::platform::launcher::{AsyncOutcomeHandler, LaunchOutcome};
use crate::platform::logging::log_line;

use super::dirs::Dirs;

fn nsurl_from_str(s: &str) -> Option<Retained<NSURL>> {
    NSURL::URLWithString(&NSString::from_str(s))
}

fn nsurl_from_path(path: &str) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(path))
}

/// Wraps a once-only [`AsyncOutcomeHandler`] in the `Fn` shape AppKit's
/// completion-handler blocks require. `block2`'s completion-handler type
/// is `dyn Fn`, not `dyn FnOnce` — AppKit calls it exactly once in
/// practice, but the binding can't express that, so `on_outcome` (itself
/// an `FnOnce`) is moved behind a `Mutex<Option<_>>` and taken out on
/// the one call that matters; a hypothetical second call becomes a
/// no-op rather than a panic. Reports
/// [`LaunchOutcome::Succeeded`]/[`LaunchOutcome::DefiniteFailure`] from
/// whether `NSError` is null; never [`LaunchOutcome::Unconfirmed`] —
/// unlike the `open`(1) watcher this module replaces
/// (`launcher::spawn_macos_open`, which can only poll a child process's
/// exit and give up past a deadline), AppKit's own completion handler
/// always eventually fires with a definite answer, so there is no
/// "still running, can't tell" state to report here.
fn outcome_completion(
    on_outcome: AsyncOutcomeHandler,
) -> impl Fn(*mut NSRunningApplication, *mut NSError) {
    let on_outcome = Mutex::new(Some(on_outcome));
    move |_app, err| {
        let outcome = if err.is_null() {
            LaunchOutcome::Succeeded
        } else {
            LaunchOutcome::DefiniteFailure
        };
        if let Some(handler) = on_outcome.lock().unwrap().take() {
            handler(outcome);
        }
    }
}

/// Opens `url` in the app bundled at `bundle_path`, with no launch
/// arguments — the sandbox-safe equivalent of `open -a <bundle> <url>`.
/// Fire-and-forget from the caller's point of view: the real
/// success/failure only arrives later, from AppKit's own completion
/// handler, which is where `on_outcome` actually runs.
pub fn open_with_app(bundle_path: &str, url: &str, on_outcome: AsyncOutcomeHandler) {
    let Some(target_url) = nsurl_from_str(url) else {
        on_outcome(LaunchOutcome::DefiniteFailure);
        return;
    };
    let app_url = nsurl_from_path(bundle_path);
    let urls = NSArray::from_slice(&[&*target_url]);
    let configuration = NSWorkspaceOpenConfiguration::new();
    let completion: block2::RcBlock<dyn Fn(*mut NSRunningApplication, *mut NSError)> =
        block2::RcBlock::new(outcome_completion(on_outcome));

    NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
        &urls,
        &app_url,
        &configuration,
        Some(&completion),
    );
}

/// Opens a custom-scheme URL (`msteams://…`, `zoommtg://…`) via
/// whichever app LaunchServices has registered for it — the
/// sandbox-safe equivalent of plain `open <url>` (no `-a`). Uses
/// `NSWorkspace.openURL:configuration:completionHandler:`, the same
/// asynchronous, completion-handler shape as [`open_with_app`] — not the
/// plain synchronous `openURL:`, which would block whichever thread
/// calls this (see this module's own doc comment) for however long the
/// round-trip to LaunchServices takes.
pub fn open_url(url: &str, on_outcome: AsyncOutcomeHandler) {
    let Some(parsed) = nsurl_from_str(url) else {
        on_outcome(LaunchOutcome::DefiniteFailure);
        return;
    };
    let configuration = NSWorkspaceOpenConfiguration::new();
    let completion: block2::RcBlock<dyn Fn(*mut NSRunningApplication, *mut NSError)> =
        block2::RcBlock::new(outcome_completion(on_outcome));

    NSWorkspace::sharedWorkspace().openURL_configuration_completionHandler(
        &parsed,
        &configuration,
        Some(&completion),
    );
}

/// Best-effort deep link straight to the "Default web browser" picker.
/// Unverified against a real sandboxed build (see the host task's own
/// notes) — the bundle identifier for the "Desktop & Dock" System
/// Settings pane, which is where that picker has lived since macOS 13.
/// If this pane id is wrong on some macOS version, the request simply
/// opens System Settings' front page instead of erroring, since
/// `openURL:` on an unrecognized `x-apple.systempreferences:` URL still
/// falls through to opening the app itself.
const SYSTEM_SETTINGS_DEFAULT_BROWSER_URL: &str =
    "x-apple.systempreferences:com.apple.Desktop-Settings.extension";

/// Requests that `app_url` (Linh's own bundle, resolved by the caller via
/// `launch_services::app_urls_for_bundle_id`) become `scheme`'s default
/// handler. Like the non-MAS `LSSetDefaultHandlerForURLScheme` path this
/// replaces, a request being *accepted* isn't the switch actually
/// happening — macOS 12+ always shows its own confirmation UI first —
/// so callers must still re-check `default_browser::is_default_browser`
/// rather than trust this call's own completion handler for the real
/// answer. That handler is only used to notice an outright *rejection*
/// (a non-null `NSError`) and, on one, fall back to pointing the user at
/// the picker directly, so the request never just silently goes
/// nowhere.
pub fn set_default_application_for_scheme(dirs: &Dirs, app_url: &str, scheme: &str) {
    let app_url = nsurl_from_path(app_url);
    let scheme = NSString::from_str(scheme);
    let dirs = dirs.clone();
    let completion: block2::RcBlock<dyn Fn(*mut NSError)> =
        block2::RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                log_line(
                    &dirs,
                    "setDefaultApplicationAtURL:toOpenURLsWithScheme: was rejected; \
                     opening System Settings' default-browser picker instead",
                );
                open_system_settings_default_browser_fallback();
            }
        });
    NSWorkspace::sharedWorkspace()
        .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
            &app_url,
            &scheme,
            Some(&completion),
        );
}

/// Runs from inside [`set_default_application_for_scheme`]'s own
/// completion handler (so already off Tauri's main thread by the time
/// this fires), but still uses the async `openURL:configuration:`
/// rather than the plain synchronous `openURL:`, for the same reason
/// every other call in this module does — see this module's own doc
/// comment — and so it never needs its own `on_outcome`/caller at all.
fn open_system_settings_default_browser_fallback() {
    let Some(url) = nsurl_from_str(SYSTEM_SETTINGS_DEFAULT_BROWSER_URL) else {
        return;
    };
    let configuration = NSWorkspaceOpenConfiguration::new();
    NSWorkspace::sharedWorkspace().openURL_configuration_completionHandler(
        &url,
        &configuration,
        None,
    );
}
