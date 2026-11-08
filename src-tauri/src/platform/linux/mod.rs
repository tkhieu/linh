//! Linux (Fedora 44, GNOME/Wayland) platform mechanics: browser detection,
//! Chromium profile discovery, config file locations and recovery, the
//! `xdg-settings`/`xdg-mime` default-browser integration, URL-argv
//! filtering, process spawning, desktop notifications and logging.

pub mod app_config;
pub mod browsers;
pub mod chromium;
pub mod default_browser;
pub mod desktop;
pub mod dirs;
pub mod launcher;
pub mod logging;
pub mod notify;
pub mod scan;
pub mod url_intake;

/// The desktop-entry id (without the `.desktop` suffix) that the RPM
/// installs Linh under. Must match `src-tauri/templates/main.desktop`'s
/// bundle output name, which CI verifies.
pub const LINH_DESKTOP_ID: &str = "Linh";
