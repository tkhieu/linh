use std::env;
use std::path::PathBuf;

/// Where Linh's config/log files live on Windows. Holds only what is
/// actually used today; unlike Linux's `Dirs`, there is no browser/app
/// discovery yet to need a home dir or scan roots — those are added back
/// once that is ported.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub config_home: PathBuf,
    pub state_home: PathBuf,
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
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_home.join("linh.toml")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_home.join("linh.log")
    }
}
