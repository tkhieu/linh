//! Desktop notifications via `osascript`(1) (built into macOS), the same
//! "shell out, fall back to logging on failure" shape Linux uses for
//! `notify-send`. `summary`/`body` are passed as separate process
//! arguments (`argv`), read by the script via `on run argv`, rather than
//! interpolated into the AppleScript source — they never run as, or
//! break out into, AppleScript code.

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::platform::launcher::{wait_bounded, BoundedWaitOutcome};
use crate::platform::logging::log_line;

use super::dirs::Dirs;

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
