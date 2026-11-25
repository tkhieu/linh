//! Desktop notifications. Native notifications are not yet implemented
//! on macOS; falls back to logging only, same as Linux does when
//! `notify-send` itself is unavailable.

use crate::platform::logging::log_line;

use super::dirs::Dirs;

// ported in a later change: native notifications need
// `UserNotifications` (or an `NSUserNotification` shim).
pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    log_line(
        dirs,
        &format!("notification (not yet supported on macOS): {summary}: {body}"),
    );
}
