use std::io;
#[cfg(target_os = "macos")]
use std::process::ExitStatus;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;
use linh_core::{Browser, Target};

use crate::platform::current::dirs::Dirs;

/// Something to run exactly once if a launch that already reported
/// synchronous success is later confirmed to have actually failed (see
/// [`spawn_detached`]'s doc comment). Only macOS ever invokes one today;
/// Linux and Windows ignore it, since their `spawn_detached` only ever
/// knows success/failure synchronously, as before.
pub type AsyncFailureHandler = Box<dyn FnOnce() + Send + 'static>;

/// The Windows registry uses `%1` and CRT quoting, and macOS launches
/// through `open(1)` rather than a `.desktop` `Exec=` line
/// (`browser.install`'s `exec` holds a bundle path there, not an
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
/// stripped first. Returns as soon as the process has been spawned
/// (`Command::spawn()` succeeding), never waiting for it to exit, on any
/// platform — this must never block the caller, which can be Tauri's
/// `setup`, the `RunEvent::Opened` listener, or the single-instance
/// callback (see `main.rs`), none of which can afford to stall.
///
/// `platform::macos::command::build_launch_command` only ever produces
/// `open`(1)-wrapped commands, for which [`spawn_macos_open`] additionally
/// starts a worker thread that watches `open`'s own exit (cheap: `open`
/// always returns quickly, whether or not the app it opens stays
/// running) and, only if that watcher *confirms* a failure — a non-zero
/// exit, or `open` itself hanging past a hard deadline — calls
/// `on_confirmed_failure` once. The plain fire-and-forget path below is
/// kept reachable for macOS too — rather than relying on the "every
/// macOS command is `open`" invariant always holding — as a safe
/// fallback for any command whose program isn't `open`, e.g. if that
/// ever changes; `on_confirmed_failure` is simply dropped unused there,
/// same as on Linux/Windows, which keep their exact existing
/// fire-and-forget behavior with no later failure detection at all.
/// `dirs` is only read on macOS, to log what the watcher found.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn spawn_detached(
    dirs: &Dirs,
    cmd: &LaunchCommand,
    on_confirmed_failure: AsyncFailureHandler,
) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    if matches!(cmd.program.as_str(), "msteams:" | "zoommtg:") {
        return super::windows::open_uri(&cmd.args[0]).map_err(io::Error::other);
    }
    #[cfg(target_os = "macos")]
    if cmd.program == super::macos::command::OPEN {
        return spawn_macos_open(dirs, cmd, on_confirmed_failure);
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

/// How long `open`(1) gets before its watcher gives up waiting for a
/// normal exit, kills it, and treats that as a confirmed failure.
#[cfg(target_os = "macos")]
const OPEN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// How often the watcher polls for `open`'s exit. Small enough to notice
/// a quick exit promptly; large enough not to spin.
#[cfg(target_os = "macos")]
const OPEN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Spawns `cmd` (an `open`(1) invocation) and returns immediately — never
/// waiting on the caller's thread — handing the child off to a worker
/// thread that watches it. `open` asks LaunchServices to open the
/// app/URL and returns on its own as soon as that request has been made,
/// without waiting for the app itself to quit (unlike a direct exec, or
/// Linux's `flatpak run`, which execs into and so blocks for the app's
/// entire lifetime) — so that watcher thread's wait is normally quick,
/// and is the only way to learn that the bundle no longer exists or no
/// handler is registered for a custom scheme (`open` succeeding at the
/// `Command::spawn()` level alone — it always exists — looks identical
/// to it actually finding the app). See [`watch_and_report`] for exactly
/// what "watches" means.
#[cfg(target_os = "macos")]
fn spawn_macos_open(
    dirs: &Dirs,
    cmd: &LaunchCommand,
    on_confirmed_failure: AsyncFailureHandler,
) -> io::Result<()> {
    let mut command = Command::new(&cmd.program);
    command
        .args(&cmd.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = command.spawn()?;
    spawn_watcher(
        child,
        OPEN_DEADLINE,
        OPEN_POLL_INTERVAL,
        dirs.clone(),
        cmd.program.clone(),
        on_confirmed_failure,
    );
    Ok(())
}

/// The subset of [`std::process::Child`] the watcher needs, so tests can
/// inject a fake instead of a real OS process.
#[cfg(target_os = "macos")]
trait Waitable {
    /// Non-blocking: `Ok(None)` while still running.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Kills and reaps (waits for) the process, so it's never left as a
    /// zombie even when forced to give up on it.
    fn kill_and_reap(&mut self) -> io::Result<()>;
}

#[cfg(target_os = "macos")]
impl Waitable for std::process::Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn kill_and_reap(&mut self) -> io::Result<()> {
        std::process::Child::kill(self)?;
        std::process::Child::wait(self).map(|_| ())
    }
}

/// Spawns exactly one worker thread that calls [`watch_and_report`] and
/// then terminates — it never lingers, however `waitable` resolves.
/// Returns immediately; the thread itself does all the waiting.
#[cfg(target_os = "macos")]
fn spawn_watcher<W: Waitable + Send + 'static>(
    waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
    dirs: Dirs,
    program: String,
    on_confirmed_failure: AsyncFailureHandler,
) {
    thread::spawn(move || {
        watch_and_report(
            waitable,
            deadline,
            poll_interval,
            &dirs,
            &program,
            on_confirmed_failure,
        );
    });
}

/// What watching `waitable` to resolution found.
#[cfg(target_os = "macos")]
enum WatchResult {
    /// Exited on its own with a successful status.
    Succeeded,
    /// Exited on its own with a failing status, or `try_wait` itself
    /// errored.
    Failed,
    /// Never exited before `deadline`; already killed and reaped.
    TimedOut,
}

/// Polls `waitable` every `poll_interval` until it exits or `deadline`
/// elapses (from this call, not from when the process actually
/// started), in which case it's killed and reaped so it's never left
/// running or as a zombie. Always terminates: either `waitable` exits
/// (success or failure) or the deadline forces it.
#[cfg(target_os = "macos")]
fn watch<W: Waitable>(
    mut waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
) -> WatchResult {
    let start = std::time::Instant::now();
    loop {
        match waitable.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    WatchResult::Succeeded
                } else {
                    WatchResult::Failed
                };
            }
            Ok(None) => {}
            Err(_) => return WatchResult::Failed,
        }
        if start.elapsed() >= deadline {
            let _ = waitable.kill_and_reap();
            return WatchResult::TimedOut;
        }
        thread::sleep(poll_interval);
    }
}

/// Runs [`watch`] and, only on a confirmed failure (not a success),
/// logs it and calls `on_confirmed_failure` exactly once. A success logs
/// nothing extra — the normal `receive->spawn` line already covers it.
#[cfg(target_os = "macos")]
fn watch_and_report<W: Waitable>(
    waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
    dirs: &Dirs,
    program: &str,
    on_confirmed_failure: AsyncFailureHandler,
) {
    match watch(waitable, deadline, poll_interval) {
        WatchResult::Succeeded => {}
        WatchResult::Failed => {
            crate::platform::logging::log_line(
                dirs,
                &format!("{program}: confirmed failed after appearing to launch; falling back"),
            );
            on_confirmed_failure();
        }
        WatchResult::TimedOut => {
            crate::platform::logging::log_line(
                dirs,
                &format!("{program}: still running after {deadline:?}; killed it and falling back"),
            );
            on_confirmed_failure();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
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

    /// Reports exactly the scripted sequence of poll results, then keeps
    /// reporting "still running" forever (so a deadline test can run it
    /// past its deadline).
    struct ScriptedWaitable {
        remaining: std::collections::VecDeque<Option<ExitStatus>>,
        kill_called: Arc<AtomicBool>,
    }

    impl Waitable for ScriptedWaitable {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(self.remaining.pop_front().flatten())
        }

        fn kill_and_reap(&mut self) -> io::Result<()> {
            self.kill_called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn still_running_forever() -> ScriptedWaitable {
        ScriptedWaitable {
            remaining: std::collections::VecDeque::new(),
            kill_called: Arc::new(AtomicBool::new(false)),
        }
    }

    fn exits_with(code: i32) -> ScriptedWaitable {
        ScriptedWaitable {
            remaining: std::collections::VecDeque::from([Some(exit_status(code))]),
            kill_called: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn zero_exit_runs_no_fallback() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_for_closure = ran.clone();
        watch_and_report(
            exits_with(0),
            Duration::from_millis(200),
            Duration::from_millis(5),
            &test_dirs(),
            "fake-open",
            Box::new(move || ran_for_closure.store(true, Ordering::SeqCst)),
        );
        assert!(
            !ran.load(Ordering::SeqCst),
            "a clean exit must not fall back"
        );
    }

    #[test]
    fn non_zero_exit_runs_fallback_exactly_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_closure = calls.clone();
        watch_and_report(
            exits_with(256), // raw wait-status 256 => exit code 1
            Duration::from_millis(200),
            Duration::from_millis(5),
            &test_dirs(),
            "fake-open",
            Box::new(move || {
                calls_for_closure.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deadline_kills_reaps_and_runs_fallback_exactly_once() {
        let waitable = still_running_forever();
        let kill_called = waitable.kill_called.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_closure = calls.clone();
        watch_and_report(
            waitable,
            Duration::from_millis(60),
            Duration::from_millis(10),
            &test_dirs(),
            "fake-open",
            Box::new(move || {
                calls_for_closure.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(
            kill_called.load(Ordering::SeqCst),
            "must kill a hung process"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// `try_wait` blocks on a two-party barrier until the test releases
    /// it. If `spawn_watcher` ever waited for the watcher thread (rather
    /// than only spawning it and returning), this test would deadlock —
    /// a hang, not a flaky timing race — since nothing can release the
    /// barrier until after `spawn_watcher` has already returned.
    struct BlocksOnBarrier {
        barrier: Arc<Barrier>,
    }

    impl Waitable for BlocksOnBarrier {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.barrier.wait();
            Ok(Some(exit_status(0)))
        }

        fn kill_and_reap(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn spawn_watcher_returns_without_waiting_for_the_child() {
        let barrier = Arc::new(Barrier::new(2));
        let waitable = BlocksOnBarrier {
            barrier: barrier.clone(),
        };
        spawn_watcher(
            waitable,
            Duration::from_secs(5),
            Duration::from_millis(10),
            test_dirs(),
            "fake-open".to_string(),
            Box::new(|| {}),
        );
        // Only reachable if `spawn_watcher` didn't block on the watcher
        // thread's `try_wait`. Releases the watcher thread so it (and
        // the test binary) can exit cleanly.
        barrier.wait();
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
