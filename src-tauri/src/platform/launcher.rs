use std::io;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;

#[cfg(target_os = "linux")]
use crate::webkit_tune;

/// Spawns `cmd` detached from Linh (never through a shell: `program` and
/// `args` are passed directly to `exec`), and reaps it in a background
/// thread so it never lingers as a zombie.
///
/// On Linux, strips [`webkit_tune::vars_set_by_linh`] from the child's
/// environment first: `webkit_tune::configure_before_tauri` sets those
/// renderer-selection vars process-wide (the only way to reach
/// WebKitGTK before Tauri launches its first `WebKitWebProcess`), so
/// without this every process Linh spawns — including unrelated
/// WebKitGTK apps like GNOME Web — would inherit Linh's own rendering
/// policy. Only vars Linh itself introduced are removed; one already
/// present in the user's own environment before Linh started is left
/// alone, since that's the user's own choice.
pub fn spawn_detached(cmd: &LaunchCommand) -> io::Result<()> {
    let mut command = Command::new(&cmd.program);
    command.args(&cmd.args);
    #[cfg(target_os = "linux")]
    strip_leaked_webkit_vars(&mut command, webkit_tune::vars_set_by_linh());
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(target_os = "linux")]
fn strip_leaked_webkit_vars(command: &mut Command, vars: &[&str]) {
    for var in vars {
        command.env_remove(var);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::OsStr;

    /// `Command::get_envs()`'s order isn't guaranteed, so tests compare
    /// as an order-independent set instead of a `Vec`.
    fn removed_vars(command: &Command) -> BTreeSet<&OsStr> {
        command
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then_some(key))
            .collect()
    }

    #[test]
    fn strips_only_the_vars_given() {
        let mut command = Command::new("true");
        strip_leaked_webkit_vars(&mut command, &["WEBKIT_DISABLE_COMPOSITING_MODE"]);
        assert_eq!(
            removed_vars(&command),
            BTreeSet::from([OsStr::new("WEBKIT_DISABLE_COMPOSITING_MODE")])
        );
    }

    #[test]
    fn strips_both_renderer_vars_when_both_were_set_by_linh() {
        let mut command = Command::new("true");
        strip_leaked_webkit_vars(
            &mut command,
            &[
                "WEBKIT_DISABLE_DMABUF_RENDERER",
                "WEBKIT_DISABLE_COMPOSITING_MODE",
            ],
        );
        assert_eq!(
            removed_vars(&command),
            BTreeSet::from([
                OsStr::new("WEBKIT_DISABLE_DMABUF_RENDERER"),
                OsStr::new("WEBKIT_DISABLE_COMPOSITING_MODE"),
            ])
        );
    }

    #[test]
    fn leaves_the_command_env_untouched_when_linh_set_no_vars() {
        let mut command = Command::new("true");
        strip_leaked_webkit_vars(&mut command, &[]);
        assert_eq!(command.get_envs().count(), 0);
    }
}
