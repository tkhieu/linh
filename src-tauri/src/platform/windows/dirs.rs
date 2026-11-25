use std::env;
use std::path::PathBuf;

/// Where Linh's config/log files live on Windows. Same shape as
/// [`crate::platform::linux::dirs::Dirs`] so the rest of `src-tauri` can
/// stay platform-agnostic; `application_dirs` is unused until Windows
/// browser/app discovery is ported.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
    pub application_dirs: Vec<PathBuf>,
}

impl Dirs {
    pub fn from_env() -> Self {
        let home = env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let appdata = env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        let local_appdata = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        Dirs {
            config_home: appdata.join("Linh"),
            state_home: local_appdata.join("Linh"),
            application_dirs: Vec::new(),
            home,
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_home.join("linh.toml")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_home.join("linh.log")
    }
}
