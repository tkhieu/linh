use std::io;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;
use linh_core::{Browser, Target};

/// The Windows registry uses `%1` and CRT quoting; other platforms keep
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
    #[cfg(not(target_os = "windows"))]
    {
        linh_core::build_launch_command(browser, target, url).map_err(|e| e.to_string())
    }
}

/// Spawns `cmd` detached from Linh (never through a shell: `program` and
/// `args` are passed directly to `exec`), and reaps it in a background
/// thread so it never lingers as a zombie.
pub fn spawn_detached(cmd: &LaunchCommand) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    if matches!(cmd.program.as_str(), "msteams:" | "zoommtg:") {
        return super::windows::open_uri(&cmd.args[0]).map_err(io::Error::other);
    }
    let mut command = Command::new(&cmd.program);
    command
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
