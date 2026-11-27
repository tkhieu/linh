//! Windows errors are logged until native toast notifications are available.

use crate::platform::logging::log_line;

use super::dirs::Dirs;

pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    log_line(
        dirs,
        &format!("notification logged (Windows toast unavailable): {summary}: {body}"),
    );
}

/// Logs routine information the user should know about but that isn't an
/// error, such as an available update. See [`notify_error`].
pub fn notify_info(dirs: &Dirs, summary: &str, body: &str) {
    log_line(
        dirs,
        &format!("notification logged (Windows toast unavailable): {summary}: {body}"),
    );
}
