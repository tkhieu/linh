//! Linux (Fedora 44, GNOME/Wayland) platform mechanics: browser detection,
//! Chromium profile discovery, config file locations and recovery, the
//! `xdg-settings`/`xdg-mime` default-browser integration, URL-argv
//! filtering, process spawning, desktop notifications and logging.

pub mod app_config;
pub mod apps;
pub mod browsers;
pub mod chromium;
pub mod default_browser;
pub mod desktop;
pub mod dirs;
pub mod icons;
pub mod notify;
pub mod scan;

/// The desktop-entry id (without the `.desktop` suffix) that the RPM
/// installs Linh under. Must match `tauri.conf.json`'s `productName`
/// (the RPM bundler names the desktop file `<productName>.desktop`),
/// which CI verifies.
pub const LINH_DESKTOP_ID: &str = "linh";
