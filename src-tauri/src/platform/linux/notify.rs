use std::process::Command;

use crate::platform::logging::log_line;

use super::dirs::Dirs;

/// Shows a desktop notification (via `notify-send`) for an error the user
/// should know about, such as a broken config file. Falls back to logging
/// only if `notify-send` is unavailable or fails.
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
    let sent = Command::new("notify-send")
        .arg("--app-name=Linh")
        .arg(summary)
        .arg(body)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !sent {
        log_line(dirs, &format!("notification failed: {summary}: {body}"));
    }
}
