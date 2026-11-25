//! Desktop notifications via `osascript`(1) (built into macOS), the same
//! "shell out, fall back to logging on failure" shape Linux uses for
//! `notify-send`. `summary`/`body` are passed as separate process
//! arguments (`argv`), read by the script via `on run argv`, rather than
//! interpolated into the AppleScript source — they never run as, or
//! break out into, AppleScript code.

use std::process::{Command, Stdio};

use crate::platform::logging::log_line;

use super::dirs::Dirs;

const OSASCRIPT: &str = "/usr/bin/osascript";
const DISPLAY_NOTIFICATION_SCRIPT: &str =
    "on run argv\n  display notification (item 2 of argv) with title (item 1 of argv)\nend run";

pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    let sent = Command::new(OSASCRIPT)
        .arg("-e")
        .arg(DISPLAY_NOTIFICATION_SCRIPT)
        .arg("--")
        .arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !sent {
        log_line(dirs, &format!("notification failed: {summary}: {body}"));
    }
}
