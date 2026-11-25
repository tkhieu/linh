//! macOS platform mechanics: LaunchServices-based browser/app discovery
//! and default-handler get/set, `.app` bundle metadata (display name,
//! icon, Chromium profile) resolution, `open`(1)-based launching, and
//! `osascript`-based notifications.

pub mod app_config;
pub mod apps;
pub mod browsers;
mod bundle;
mod chromium;
pub mod command;
pub mod default_browser;
pub mod dirs;
pub mod icons;
mod launch_services;
pub mod notify;

/// Linh's `CFBundleIdentifier`, as declared in `tauri.conf.json`'s
/// top-level `identifier` (which the macOS bundler writes into the
/// built app's `Info.plist`). Must stay in sync with that value.
pub const LINH_BUNDLE_ID: &str = "dev.tkhieu.linh";
