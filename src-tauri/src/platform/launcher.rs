use std::io;
#[cfg(target_os = "macos")]
use std::process::ExitStatus;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;
use linh_core::{Browser, Target};

use crate::platform::current::dirs::Dirs;

/// The Windows registry uses `%1` and CRT quoting, and macOS launches
/// through `open(1)`/a direct bundle exec rather than a `.desktop`
/// `Exec=` line (`browser.install`'s `exec` holds a bundle path there,
/// not an `Exec=` string) — both need their own command builder. Linux
/// keeps the existing Desktop Entry launch parser.
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
///
/// `platform::macos::command::build_launch_command` only ever produces
/// `open`(1)-wrapped commands, which are spawned differently — see
/// [`spawn_macos_open`] — since it's the one case where waiting for the
/// child's exit is both safe and useful (`open` always returns quickly,
/// whether or not the app it opens stays running). The plain
/// fire-and-forget path below is kept reachable for macOS too — rather
/// than relying on that always being true — as a safe fallback for any
/// command whose program isn't `open`, e.g. if that invariant ever
/// changes. `dirs` is only read on macOS, to log a bounded-wait timeout
/// — see [`spawn_macos_open`].
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn spawn_detached(dirs: &Dirs, cmd: &LaunchCommand) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    if matches!(cmd.program.as_str(), "msteams:" | "zoommtg:") {
        return super::windows::open_uri(&cmd.args[0]).map_err(io::Error::other);
    }
    #[cfg(target_os = "macos")]
    if cmd.program == super::macos::command::OPEN {
        return spawn_macos_open(dirs, cmd);
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
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        // Puts the child in its own process group so it's never tied to
        // Linh's: if Linh's own process group ever gets a signal (e.g. a
        // terminal session closing, in a dev build run from one), the
        // browser the child becomes must not be killed along with it.
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// How long to wait for `open(1)` to exit before giving up on getting a
/// real success/failure signal from it.
#[cfg(target_os = "macos")]
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `open(1)` asks LaunchServices to open the app/URL and returns as soon
/// as that request has been made, without waiting for the app itself to
/// quit (unlike a direct exec, or Linux's `flatpak run`, which execs
/// into and so blocks for the app's entire lifetime) — so waiting for
/// its exit status is normally cheap, and is the only way to learn that
/// the bundle no longer exists or no handler is registered for a custom
/// scheme, so the caller (`routing::open_app_target_with`) can fall back
/// to the configured browser instead of silently dropping the link.
/// `open` succeeding at the `Command::spawn()` level alone (it always
/// exists) would otherwise look identical to it actually finding the
/// app.
///
/// This still runs the wait on a worker thread with a bounded timeout,
/// rather than on the caller's thread (routing, which can run from
/// Tauri's setup/event-loop callbacks — see `main.rs`'s `RunEvent`
/// listener and single-instance callback), so a LaunchServices request
/// that never completes can never freeze Linh. A timeout does *not*
/// trigger the browser fallback the way a real (non-zero exit) failure
/// does: `open` may still be about to succeed, and falling back here as
/// well risks the link opening twice (once whenever `open` actually
/// finishes, once in the fallback browser) — logging and otherwise
/// treating it as success is the safer of the two imperfect choices.
#[cfg(target_os = "macos")]
fn spawn_macos_open(dirs: &Dirs, cmd: &LaunchCommand) -> io::Result<()> {
    let program = cmd.program.clone();
    let args = cmd.args.clone();
    let outcome = run_with_timeout(OPEN_TIMEOUT, move || {
        Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    });
    handle_open_outcome(dirs, &cmd.program, outcome)
}

/// What came back from running a bounded-wait command: it finished (with
/// the usual `io::Result<ExitStatus>`), or the timeout elapsed first and
/// it may still be running.
#[cfg(target_os = "macos")]
enum RunOutcome {
    Finished(io::Result<ExitStatus>),
    TimedOut,
}

/// Runs `run` on a worker thread and waits up to `timeout` for it,
/// without ever blocking the calling thread past `timeout`. `run` keeps
/// running in its own thread past a timeout (reaping whatever child it
/// spawned when it eventually does finish); its result is simply
/// dropped, since nothing is listening for it anymore.
#[cfg(target_os = "macos")]
fn run_with_timeout<F>(timeout: std::time::Duration, run: F) -> RunOutcome
where
    F: FnOnce() -> io::Result<ExitStatus> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(run());
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => RunOutcome::Finished(result),
        Err(
            std::sync::mpsc::RecvTimeoutError::Timeout
            | std::sync::mpsc::RecvTimeoutError::Disconnected,
        ) => RunOutcome::TimedOut,
    }
}

/// Turns a [`RunOutcome`] for `program` into the `io::Result` callers
/// expect: success/failure map straight through; a timeout logs why it's
/// *not* being treated as a failure (see [`spawn_macos_open`]) and
/// counts as success.
#[cfg(target_os = "macos")]
fn handle_open_outcome(dirs: &Dirs, program: &str, outcome: RunOutcome) -> io::Result<()> {
    match outcome {
        RunOutcome::Finished(Ok(status)) if status.success() => Ok(()),
        RunOutcome::Finished(Ok(status)) => {
            Err(io::Error::other(format!("{program} exited with {status}")))
        }
        RunOutcome::Finished(Err(err)) => Err(err),
        RunOutcome::TimedOut => {
            crate::platform::logging::log_line(
                dirs,
                &format!(
                    "{program} had not exited after {OPEN_TIMEOUT:?}; not treating this as a \
                     launch failure (it may still succeed) and not falling back, since that \
                     could open the link twice — continuing without waiting further"
                ),
            );
            Ok(())
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::time::Duration;

    fn exit_status(code: i32) -> ExitStatus {
        ExitStatusExt::from_raw(code)
    }

    fn test_dirs() -> Dirs {
        let root = std::env::temp_dir().join(format!(
            "linh-launcher-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Dirs {
            config_home: root.join("config"),
            state_home: root.join("state"),
        }
    }

    #[test]
    fn finishing_before_the_timeout_is_reported_as_finished() {
        let outcome = run_with_timeout(Duration::from_millis(200), || Ok(exit_status(0)));
        assert!(matches!(outcome, RunOutcome::Finished(Ok(status)) if status.success()));
    }

    #[test]
    fn exceeding_the_timeout_is_reported_as_timed_out() {
        let outcome = run_with_timeout(Duration::from_millis(50), || {
            thread::sleep(Duration::from_millis(300));
            Ok(exit_status(0))
        });
        assert!(matches!(outcome, RunOutcome::TimedOut));
    }

    #[test]
    fn successful_exit_is_ok() {
        let dirs = test_dirs();
        assert!(
            handle_open_outcome(&dirs, "open", RunOutcome::Finished(Ok(exit_status(0)))).is_ok()
        );
    }

    #[test]
    fn failing_exit_is_err() {
        let dirs = test_dirs();
        assert!(
            handle_open_outcome(&dirs, "open", RunOutcome::Finished(Ok(exit_status(256)))).is_err()
        );
    }

    #[test]
    fn spawn_error_is_propagated() {
        let dirs = test_dirs();
        let err = handle_open_outcome(
            &dirs,
            "open",
            RunOutcome::Finished(Err(io::Error::other("boom"))),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn timing_out_is_ok_not_a_failure() {
        // A timeout must never be treated as a launch failure: doing so
        // would make the caller fall back to another browser while
        // `open` might still succeed on its own, risking the link
        // opening twice.
        let dirs = test_dirs();
        assert!(handle_open_outcome(&dirs, "open", RunOutcome::TimedOut).is_ok());
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
