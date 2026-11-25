use std::io;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;
use linh_core::{Browser, Target};

/// The Windows registry uses `%1` and CRT quoting, and macOS launches
/// through `open -a <bundle> --args ...` rather than a `.desktop` `Exec=`
/// line (`browser.install`'s `exec` holds a bundle path there, not an
/// `Exec=` string) — both need their own command builder. Linux keeps
/// the existing Desktop Entry launch parser.
pub fn build_launch_command(
    browser: &Browser,
    target: &Target,
    url: &str,
) -> Result<LaunchCommand, String> {
    #[cfg(target_os = "windows")]
    {
        super::windows::command::build_launch_command(browser, target, url)
    }
    #[cfg(target_os = "macos")]
    {
        super::macos::command::build_launch_command(browser, target, url)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        linh_core::build_launch_command(browser, target, url).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "linux")]
use crate::webkit_tune;

/// Extension trait on [`Command`] so every child process Linh spawns on
/// Linux — browsers, app targets, `xdg-settings`/`xdg-mime`,
/// `notify-send` — strips [`webkit_tune::vars_set_by_linh`] from its
/// environment by chaining one call right after `Command::new`, instead
/// of relying on each call site to remember a separate step.
/// `webkit_tune::configure_before_tauri` sets those renderer-selection
/// vars process-wide (the only way to reach WebKitGTK before Tauri
/// launches its first `WebKitWebProcess`), so without this every child
/// process — including unrelated WebKitGTK apps like GNOME Web — would
/// inherit Linh's own rendering policy. Only vars Linh itself introduced
/// are removed; one already present in the user's own environment
/// before Linh started is left alone, since that's the user's own
/// choice. A no-op on other platforms.
pub trait StripLinhWebkitEnv {
    fn strip_linh_webkit_env(&mut self) -> &mut Self;
}

impl StripLinhWebkitEnv for Command {
    #[cfg(target_os = "linux")]
    fn strip_linh_webkit_env(&mut self) -> &mut Self {
        strip_vars(self, webkit_tune::vars_set_by_linh());
        self
    }

    #[cfg(not(target_os = "linux"))]
    fn strip_linh_webkit_env(&mut self) -> &mut Self {
        self
    }
}

#[cfg(target_os = "linux")]
fn strip_vars(command: &mut Command, vars: &[&str]) {
    for var in vars {
        command.env_remove(var);
    }
}

/// Spawns `cmd` detached from Linh (never through a shell: `program` and
/// `args` are passed directly to `exec`), and reaps it in a background
/// thread so it never lingers as a zombie. See
/// [`StripLinhWebkitEnv`]'s doc comment for why its environment is
/// stripped first.
pub fn spawn_detached(cmd: &LaunchCommand) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    if matches!(cmd.program.as_str(), "msteams:" | "zoommtg:") {
        return super::windows::open_uri(&cmd.args[0]).map_err(io::Error::other);
    }
    let mut command = Command::new(&cmd.program);
    command
        .strip_linh_webkit_env()
        .args(&cmd.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
        strip_vars(&mut command, &["WEBKIT_DISABLE_COMPOSITING_MODE"]);
        assert_eq!(
            removed_vars(&command),
            BTreeSet::from([OsStr::new("WEBKIT_DISABLE_COMPOSITING_MODE")])
        );
    }

    #[test]
    fn strips_both_renderer_vars_when_both_were_set_by_linh() {
        let mut command = Command::new("true");
        strip_vars(
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
        strip_vars(&mut command, &[]);
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn strip_linh_webkit_env_chains_from_command_new() {
        // Exercises the public trait method itself (not just the private
        // helper above), against whatever `vars_set_by_linh` reports in
        // this test binary — exercising the call-site ergonomics the
        // trait exists for.
        let mut command = Command::new("true");
        command.strip_linh_webkit_env().arg("-n");
        let expected: BTreeSet<&OsStr> = webkit_tune::vars_set_by_linh()
            .iter()
            .map(|v| OsStr::new(*v))
            .collect();
        assert_eq!(removed_vars(&command), expected);
    }
}
