//! `linh-core`: pure-Rust routing logic for Linh (config model, URL
//! pattern matching, routing decisions, browser model, launch-arg
//! builder). No Tauri or UI dependencies, so it is testable in isolation
//! and shared across all target platforms.

pub mod browser;
pub mod config;
pub mod launch;
pub mod matching;
pub mod routing;

pub use browser::{Browser, BrowserFamily, BrowserProfile, InstallKind};
pub use config::{Config, ConfigError, OpenIn, PromptMode, Rule, Target};
pub use launch::{build_launch_command, LaunchCommand};
pub use matching::pattern_matches;
pub use routing::{decide, Decision, PlatformCaps};
