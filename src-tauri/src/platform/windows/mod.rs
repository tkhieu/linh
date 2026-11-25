//! Windows platform mechanics. Not yet implemented — every function
//! here returns an explicit, honest "unsupported" result (`None`,
//! `false`, an empty list, or `Err`) rather than a guess, so the UI's
//! existing error handling (it already copes with no browsers detected,
//! a failed default-browser set, a missing icon, ...) covers it until a
//! later change ports this module for real.

pub mod app_config;
pub mod apps;
pub mod browsers;
pub mod default_browser;
pub mod dirs;
pub mod icons;
pub mod notify;
