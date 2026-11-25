//! Desktop notifications. Native notifications are not yet implemented
//! on Windows; falls back to logging only, same as Linux does when
//! `notify-send` itself is unavailable.

use crate::platform::logging::log_line;

use super::dirs::Dirs;

// ported in a later change: native notifications need the Windows
// `ToastNotification` API.
pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    log_line(
        dirs,
        &format!("notification (not yet supported on Windows): {summary}: {body}"),
    );
}
