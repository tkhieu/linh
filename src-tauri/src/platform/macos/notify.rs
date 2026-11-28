//! Desktop notifications.
//!
//! The default (`github`, etc.) macOS build shells out to `osascript`(1)
//! (built into macOS), the same "shell out, fall back to logging on
//! failure" shape Linux uses for `notify-send`. `summary`/`body` are
//! passed as separate process arguments (`argv`), read by the script via
//! `on run argv`, rather than interpolated into the AppleScript source —
//! they never run as, or break out into, AppleScript code.
//!
//! The Mac App Store build (`LINH_DIST=mas`) can't exec `osascript`(1)
//! at all (no exec rights over arbitrary system binaries, same
//! restriction `command.rs`/`icons.rs` work around), so it goes through
//! `tauri-plugin-notification` instead — the same real, native-toast
//! path `updates::notify_via_toast` already uses for Windows. That needs
//! an `AppHandle`, which these two functions' callers (some of them,
//! like `AppState::init`, running before `main.rs`'s `setup` closure has
//! finished its own early setup) don't have to hand down; [`set_app_handle`]
//! is how `main.rs` hands one over instead, stashed in a process-wide
//! [`OnceLock`] rather than threaded through every call site.

use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::platform::launcher::{wait_bounded, BoundedWaitOutcome};
use crate::platform::logging::log_line;
use crate::updates::{current_dist, Dist};

use super::dirs::Dirs;

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Must be called once from `main.rs`'s `setup` closure, before
/// `AppState::init()` (whose own notify calls are the earliest in the
/// process) — harmless to call more than once; only the first handle is
/// kept. A no-op on every non-MAS build's notification path, which never
/// reads [`APP_HANDLE`] at all.
pub fn set_app_handle(handle: AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

const OSASCRIPT: &str = "/usr/bin/osascript";
const DISPLAY_NOTIFICATION_SCRIPT: &str =
    "on run argv\n  display notification (item 2 of argv) with title (item 1 of argv)\nend run";

/// How long `osascript` gets to display the notification before Linh
/// gives up on it — `notify_error` runs on whichever thread hit the
/// notification-worthy condition (routing, startup), none of which may
/// stall indefinitely on a wedged AppleScript helper. Bounded (and, if
/// it's still alive past this, killed and reaped) the same way as
/// `platform::launcher`'s own process watcher, via the shared
/// [`wait_bounded`] — never a second, duplicate implementation.
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often that wait polls for `osascript`'s exit.
const OSASCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    send(dirs, summary, body);
}

/// Shows a desktop notification for routine information the user should
/// know about but that isn't an error, such as an available update. Same
/// delivery and fallback as [`notify_error`].
pub fn notify_info(dirs: &Dirs, summary: &str, body: &str) {
    send(dirs, summary, body);
}

fn send(dirs: &Dirs, summary: &str, body: &str) {
    if current_dist() == Dist::Mas {
        send_mas(dirs, summary, body);
        return;
    }
    send_osascript(dirs, summary, body);
}

/// `.show()` returning `Ok` only means the OS *accepted the request to
/// schedule* the toast, not that it was actually displayed (same caveat
/// as `updates::notify_via_toast`'s own doc comment) — logged either way
/// so there is always a durable record, same as the osascript path's own
/// `log_line` fallback below. Logs instead of showing a toast when
/// [`APP_HANDLE`] isn't set yet — only possible for a notification fired
/// before `main.rs`'s `setup` closure calls [`set_app_handle`], which
/// today is nothing past `AppState::init`'s own first lines.
fn send_mas(dirs: &Dirs, summary: &str, body: &str) {
    let shown = APP_HANDLE.get().is_some_and(|handle| {
        handle
            .notification()
            .builder()
            .title(summary)
            .body(body)
            .show()
            .is_ok()
    });
    log_line(
        dirs,
        &format!(
            "notification {}: {summary}: {body}",
            if shown {
                "toasted"
            } else {
                "logged (no app handle yet, or the toast failed)"
            }
        ),
    );
}

fn send_osascript(dirs: &Dirs, summary: &str, body: &str) {
    let sent = Command::new(OSASCRIPT)
        .arg("-e")
        .arg(DISPLAY_NOTIFICATION_SCRIPT)
        .arg("--")
        .arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|child| {
            matches!(
                wait_bounded(child, OSASCRIPT_TIMEOUT, OSASCRIPT_POLL_INTERVAL),
                BoundedWaitOutcome::Exited(status) if status.success()
            )
        })
        .unwrap_or(false);

    if !sent {
        log_line(dirs, &format!("notification failed: {summary}: {body}"));
    }
}
