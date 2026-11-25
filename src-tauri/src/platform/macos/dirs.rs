use std::env;
use std::path::PathBuf;

/// Where Linh's config/log files live on macOS. Holds only what is
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
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Dirs {
            config_home: home.join("Library/Application Support/Linh"),
            state_home: home.join("Library/Logs/Linh"),
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_home.join("linh.toml")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_home.join("linh.log")
    }
}
