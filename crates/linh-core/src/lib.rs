//! `linh-core`: pure-Rust routing logic for Linh (config model, URL
//! pattern matching, routing decisions, browser model, launch-arg
//! builder). No Tauri or UI dependencies, so it is testable in isolation
//! and shared across all target platforms.

pub mod browser;
pub mod config;
pub mod launch;
pub mod matching;
pub mod presets;
pub mod routing;
pub mod zoom;

pub use browser::{Browser, BrowserFamily, BrowserProfile, InstallKind};
pub use config::{
    load_config, parse_config, save_config, serialize_config, AppTarget, Config, ConfigError,
    OpenIn, PromptMode, Rule, Target,
};
pub use launch::{build_launch_command, LaunchCommand, LaunchError};
pub use matching::pattern_matches;
pub use presets::{Preset, PresetDef, PRESET_DEFS};
pub use routing::{decide, Decision, PlatformCaps};
pub use zoom::zoom_app_url;
