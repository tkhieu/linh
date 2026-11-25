use std::process::Command;

use crate::platform::launcher::StripLinhWebkitEnv;
use crate::platform::logging::log_line;

use super::dirs::Dirs;

/// Shows a desktop notification (via `notify-send`) for an error the user
/// should know about, such as a broken config file. Falls back to logging
/// only if `notify-send` is unavailable or fails.
pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    let sent = Command::new("notify-send")
        .strip_linh_webkit_env()
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
