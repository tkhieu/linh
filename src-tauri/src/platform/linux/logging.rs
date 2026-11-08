use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use super::dirs::Dirs;

/// Logs `msg` to stderr and appends it (best-effort) to
/// `~/.local/state/linh/linh.log`, each line prefixed with a Unix
/// millisecond timestamp.
pub fn log_line(dirs: &Dirs, msg: &str) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("[{millis}] {msg}");
    eprintln!("{line}");

    let path = dirs.log_path();
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}
