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
pub const LINH_BUNDLE_ID: &str = "dev.hieutk.linh";

/// The `CFBundleIdentifier`/app id Linh shipped under before the
/// `dev.tkhieu.linh` -> `dev.hieutk.linh` rename (Flathub requires the id
/// to match a domain the owner controls). Still checked wherever Linh
/// recognizes *itself* — self-exclusion from its own browser list, the
/// Linux desktop-id self-exclusion — so a leftover registration under the
/// old id is never mistaken for a different, installable browser. It must
/// NOT be treated as "Linh" by [`default_browser::is_default_browser`]:
/// LaunchServices keys the default handler by bundle id, so an install
/// that was default under the old id needs the user to pick Linh again
/// after upgrading, and that must surface as "not default", not silently
/// stay "default".
pub const LINH_BUNDLE_ID_LEGACY: &str = "dev.tkhieu.linh";
