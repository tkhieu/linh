use std::env;
use std::path::PathBuf;

/// Where Linh's config/log files live on macOS. Same shape as
/// [`crate::platform::linux::dirs::Dirs`] so the rest of `src-tauri` can
/// stay platform-agnostic; `application_dirs` is unused until macOS
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
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Dirs {
            config_home: home.join("Library/Application Support/Linh"),
            state_home: home.join("Library/Logs/Linh"),
            application_dirs: vec![PathBuf::from("/Applications")],
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
