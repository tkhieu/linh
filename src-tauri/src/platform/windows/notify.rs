//! Windows errors are logged until native toast notifications are available.

use crate::platform::logging::log_line;

use super::dirs::Dirs;

pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    log_line(
        dirs,
        &format!("notification logged (Windows toast unavailable): {summary}: {body}"),
    );
}
