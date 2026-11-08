use serde::{Deserialize, Serialize};

/// Browser engine family, which determines which launch flags apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserFamily {
    Chromium,
    Firefox,
    Other,
}

/// A browser profile: the on-disk directory name and a human-readable
/// display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub dir: String,
    pub display_name: String,
}

/// How the browser is installed on this system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// A natively installed binary; `exec` is the `Exec=` line from its
    /// `.desktop` file (which may include field codes such as `%u`).
    Native { exec: String },
    /// A Flatpak application, launched via `flatpak run <app_id>`.
    Flatpak { app_id: String },
}

/// A detected, installed browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Browser {
    pub id: String,
    pub display_name: String,
    pub family: BrowserFamily,
    pub install: InstallKind,
    #[serde(default)]
    pub profiles: Vec<BrowserProfile>,
    pub supports_private: bool,
}
