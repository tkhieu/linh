use std::env;
use std::path::PathBuf;

/// The XDG base directories relevant to Linh, resolved once and threaded
/// through the rest of the platform module instead of read ad hoc. Tests
/// construct this directly with temp-dir paths so no test ever touches the
/// real `$HOME`.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
    /// `.desktop` directories to scan for browsers, in override order:
    /// system dirs first (lowest priority), then the user's Flatpak
    /// exports, then the user's own `applications` dir (highest
    /// priority) — a later entry wins on a duplicate desktop id, so a
    /// user-level entry always overrides a system one. Defaults to the
    /// real system/XDG locations in [`Dirs::from_env`]; tests override
    /// this with temp-dir paths so no test ever reads the real
    /// `/usr/share/applications` or `$HOME`.
    pub application_dirs: Vec<PathBuf>,
}

impl Dirs {
    /// Resolves the real XDG directories from the process environment.
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let state_home = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        let data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        // Override precedence (later wins on a duplicate desktop id):
        // system dirs lowest, then the user's own Flatpak exports, then
        // the user's own `applications` dir highest — a user-level entry
        // always overrides a system entry with the same id.
        let application_dirs = vec![
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
            PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            data_home.join("flatpak/exports/share/applications"),
            data_home.join("applications"),
        ];
        Dirs {
            home,
            config_home,
            state_home,
            application_dirs,
        }
    }

    /// `~/.config/linh/linh.toml` (XDG_CONFIG_HOME aware).
    pub fn config_path(&self) -> PathBuf {
        self.config_home.join("linh").join("linh.toml")
    }

    /// `~/.local/state/linh/linh.log` (XDG_STATE_HOME aware).
    pub fn log_path(&self) -> PathBuf {
        self.state_home.join("linh").join("linh.log")
    }
}
