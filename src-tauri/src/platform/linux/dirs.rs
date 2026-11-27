use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

/// The XDG base directories relevant to Linh, resolved once and threaded
/// through the rest of the platform module instead of read ad hoc. Tests
/// construct this directly with temp-dir paths, or exercise [`Dirs::resolve`]
/// with explicit env values, so no test ever touches the real `$HOME` or
/// mutates process-wide env vars (which races under parallel test
/// execution).
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
    /// `.desktop` directories to scan for browsers, in override order:
    /// system dirs first (lowest priority), then the user's own
    /// `~/.local/share` Flatpak-exports and `applications` dirs (always
    /// scanned, regardless of `XDG_DATA_HOME`), then — only when
    /// `XDG_DATA_HOME` is validly set to something other than
    /// `~/.local/share` — that custom data dir's equivalents, highest
    /// priority. A later entry wins on a duplicate desktop id, so a
    /// user-level entry always overrides a system one. Defaults to the
    /// real system/XDG locations in [`Dirs::from_env`]; tests override
    /// this with temp-dir paths so no test ever reads the real
    /// `/usr/share/applications` or `$HOME`.
    pub application_dirs: Vec<PathBuf>,
}

impl Dirs {
    /// Resolves the real XDG directories from the process environment.
    pub fn from_env() -> Self {
        Self::resolve(
            env::var_os("HOME"),
            env::var_os("XDG_CONFIG_HOME"),
            env::var_os("XDG_STATE_HOME"),
            env::var_os("XDG_DATA_HOME"),
        )
    }

    /// The pure resolution logic behind [`Dirs::from_env`], taking the raw
    /// env values explicitly. An `XDG_*` value that is unset, empty, or
    /// not an absolute path is ignored in favor of the XDG default
    /// (`~/.config`, `~/.local/state`, `~/.local/share`) — never a path
    /// relative to the current working directory.
    fn resolve(
        home_var: Option<OsString>,
        config_home_var: Option<OsString>,
        state_home_var: Option<OsString>,
        data_home_var: Option<OsString>,
    ) -> Self {
        let home = home_var
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));

        let default_config_home = home.join(".config");
        let config_home = valid_absolute_path(config_home_var).unwrap_or(default_config_home);

        let default_state_home = home.join(".local/state");
        let state_home = valid_absolute_path(state_home_var).unwrap_or(default_state_home);

        let default_data_home = home.join(".local/share");
        let data_home =
            valid_absolute_path(data_home_var).unwrap_or_else(|| default_data_home.clone());

        // The literal `~/.local/share` Flatpak-exports and `applications`
        // dirs are always scanned, regardless of `XDG_DATA_HOME`.
        let mut application_dirs = vec![
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
            PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            default_data_home.join("flatpak/exports/share/applications"),
            default_data_home.join("applications"),
        ];
        if data_home != default_data_home {
            application_dirs.push(data_home.join("flatpak/exports/share/applications"));
            application_dirs.push(data_home.join("applications"));
        }
        dedupe_preserving_order(&mut application_dirs);

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

    /// `~/.local/state/linh/updates.json` (XDG_STATE_HOME aware): the
    /// persisted update-notification state (see `crate::updates`).
    pub fn updates_state_path(&self) -> PathBuf {
        self.state_home.join("linh").join("updates.json")
    }
}

/// Returns `Some(path)` only when `val` is set, non-empty, and an
/// absolute path; otherwise `None` (the caller falls back to the XDG
/// default).
fn valid_absolute_path(val: Option<OsString>) -> Option<PathBuf> {
    let val = val?;
    if val.is_empty() {
        return None;
    }
    let path = PathBuf::from(val);
    path.is_absolute().then_some(path)
}

/// Removes duplicate paths, keeping each one at the position of its
/// *last* occurrence (highest precedence) rather than its first, while
/// preserving the relative order of the entries that remain. This matters
/// when a custom `XDG_DATA_HOME` happens to equal a fixed system/default
/// dir: the custom entry must still end up last (highest precedence),
/// not silently dropped in favor of the earlier, lower-priority one.
fn dedupe_preserving_order(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    let mut kept_reversed = Vec::with_capacity(paths.len());
    for path in paths.drain(..).rev() {
        if seen.insert(path.clone()) {
            kept_reversed.push(path);
        }
    }
    kept_reversed.reverse();
    *paths = kept_reversed;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn unset_xdg_vars_fall_back_to_defaults() {
        let dirs = Dirs::resolve(Some(os("/home/u")), None, None, None);
        assert_eq!(dirs.config_home, PathBuf::from("/home/u/.config"));
        assert_eq!(dirs.state_home, PathBuf::from("/home/u/.local/state"));
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
            ]
        );
    }

    #[test]
    fn empty_xdg_vars_fall_back_to_defaults() {
        let dirs = Dirs::resolve(
            Some(os("/home/u")),
            Some(os("")),
            Some(os("")),
            Some(os("")),
        );
        assert_eq!(dirs.config_home, PathBuf::from("/home/u/.config"));
        assert_eq!(dirs.state_home, PathBuf::from("/home/u/.local/state"));
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
            ]
        );
    }

    #[test]
    fn relative_xdg_vars_fall_back_to_defaults() {
        // A relative value must never make Linh write config/state next
        // to the current working directory.
        let dirs = Dirs::resolve(
            Some(os("/home/u")),
            Some(os("relative/config")),
            Some(os("relative/state")),
            Some(os("relative/data")),
        );
        assert_eq!(dirs.config_home, PathBuf::from("/home/u/.config"));
        assert_eq!(dirs.state_home, PathBuf::from("/home/u/.local/state"));
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
            ]
        );
    }

    #[test]
    fn valid_absolute_xdg_config_and_state_home_are_used() {
        let dirs = Dirs::resolve(
            Some(os("/home/u")),
            Some(os("/custom/config")),
            Some(os("/custom/state")),
            None,
        );
        assert_eq!(dirs.config_home, PathBuf::from("/custom/config"));
        assert_eq!(dirs.state_home, PathBuf::from("/custom/state"));
    }

    #[test]
    fn custom_data_home_adds_itself_highest_while_keeping_default_local_share() {
        let dirs = Dirs::resolve(Some(os("/home/u")), None, None, Some(os("/custom/data")));
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
                PathBuf::from("/custom/data/flatpak/exports/share/applications"),
                PathBuf::from("/custom/data/applications"),
            ]
        );
    }

    #[test]
    fn xdg_data_home_equal_to_default_does_not_duplicate_entries() {
        let dirs = Dirs::resolve(
            Some(os("/home/u")),
            None,
            None,
            Some(os("/home/u/.local/share")),
        );
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
            ]
        );
    }

    #[test]
    fn xdg_data_home_equal_to_a_fixed_dir_keeps_its_highest_precedence() {
        // XDG_DATA_HOME="/usr/share" makes the custom "applications" dir
        // collide with the fixed, lowest-priority "/usr/share/applications"
        // entry. The user explicitly set XDG_DATA_HOME, so that path must
        // still end up *last* (highest precedence), appearing exactly
        // once, not silently kept at its original lowest-priority spot.
        let dirs = Dirs::resolve(Some(os("/home/u")), None, None, Some(os("/usr/share")));
        let target = PathBuf::from("/usr/share/applications");
        let occurrences = dirs
            .application_dirs
            .iter()
            .filter(|p| *p == &target)
            .count();
        assert_eq!(occurrences, 1, "{target:?} must appear exactly once");
        assert_eq!(
            dirs.application_dirs.last(),
            Some(&target),
            "the custom XDG_DATA_HOME entry must be last (highest precedence)"
        );
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
                PathBuf::from("/usr/share/flatpak/exports/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn xdg_data_home_equal_to_user_flatpak_export_parent_keeps_highest_precedence() {
        // XDG_DATA_HOME="~/.local/share/flatpak/exports/share" makes its
        // "applications" entry collide with the always-scanned default
        // "~/.local/share/flatpak/exports/share/applications" entry. The
        // explicit XDG_DATA_HOME setting must still win that path's
        // position (last / highest precedence), appearing exactly once.
        let dirs = Dirs::resolve(
            Some(os("/home/u")),
            None,
            None,
            Some(os("/home/u/.local/share/flatpak/exports/share")),
        );
        let target = PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications");
        let occurrences = dirs
            .application_dirs
            .iter()
            .filter(|p| *p == &target)
            .count();
        assert_eq!(occurrences, 1, "{target:?} must appear exactly once");
        assert_eq!(
            dirs.application_dirs.last(),
            Some(&target),
            "the custom XDG_DATA_HOME entry must be last (highest precedence)"
        );
        assert_eq!(
            dirs.application_dirs,
            vec![
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/home/u/.local/share/applications"),
                PathBuf::from(
                    "/home/u/.local/share/flatpak/exports/share/flatpak/exports/share/applications"
                ),
                PathBuf::from("/home/u/.local/share/flatpak/exports/share/applications"),
            ]
        );
    }
}
