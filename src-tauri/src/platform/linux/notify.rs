use std::process::Command;

use super::dirs::Dirs;
use super::logging::log_line;

/// Shows a desktop notification (via `notify-send`) for an error the user
/// should know about, such as a broken config file. Falls back to logging
/// only if `notify-send` is unavailable or fails.
pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
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
