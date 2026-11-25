//! Windows registry, protocol, icon, and default-app integration.

pub mod app_config;
pub mod apps;
pub mod browsers;
pub mod command;
pub mod default_browser;
pub mod dirs;
pub mod icons;
pub mod notify;
mod registry;
mod shell;
pub use shell::open_uri;
