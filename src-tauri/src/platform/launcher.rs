use std::io;
#[cfg(target_os = "macos")]
use std::process::ExitStatus;
use std::process::{Command, Stdio};
use std::thread;

use linh_core::LaunchCommand;
use linh_core::{Browser, Target};

use crate::platform::current::dirs::Dirs;

/// What a launch that already reported synchronous success turned out
/// to do, confirmed later (see [`spawn_detached`]'s doc comment). Only
/// macOS's watcher ever constructs one of these; Linux and Windows never
/// invoke the handler at all, since their `spawn_detached` only ever
/// knows success/failure synchronously, as before — so on those
/// platforms nothing ever constructs a value of this type, only
/// `routing.rs`'s handlers match against it (as they must, to stay
/// platform-agnostic code that still type-checks everywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum LaunchOutcome {
    /// Exited on its own with a successful status.
    Succeeded,
    /// Exited on its own with a failing status: a definite failure, safe
    /// to fall back from (the original request was never duplicated).
    DefiniteFailure,
    /// Could not be confirmed either way — `try_wait` itself errored, or
    /// the process was still running past a hard deadline and had to be
    /// killed. Must never be treated as a failure worth falling back
    /// from: the original launch may still have succeeded (or even
    /// still be running), and opening a fallback on top of that risks
    /// the link opening twice.
    Unconfirmed,
}

/// Runs exactly once with the launch's [`LaunchOutcome`], once known.
pub type AsyncOutcomeHandler = Box<dyn FnOnce(LaunchOutcome) + Send + 'static>;

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
/// running) and reports the [`LaunchOutcome`] it finds to `on_outcome`
/// once. The plain fire-and-forget path below is kept reachable for
/// macOS too — rather than relying on the "every macOS command is
/// `open`" invariant always holding — as a safe fallback for any command
/// whose program isn't `open`, e.g. if that ever changes; `on_outcome`
/// is simply dropped unused there, same as on Linux/Windows, which keep
/// their exact existing fire-and-forget behavior with no later outcome
/// detection at all. `dirs` is only read on macOS, to log what the
/// watcher found.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn spawn_detached(
    dirs: &Dirs,
    cmd: &LaunchCommand,
    on_outcome: AsyncOutcomeHandler,
) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    if matches!(cmd.program.as_str(), "msteams:" | "zoommtg:") {
        return super::windows::open_uri(&cmd.args[0]).map_err(io::Error::other);
    }
    #[cfg(target_os = "macos")]
    if cmd.program == super::macos::command::OPEN {
        return spawn_macos_open(dirs, cmd, on_outcome);
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
    on_outcome: AsyncOutcomeHandler,
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
        on_outcome,
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
    /// zombie even when forced to give up on it. Tolerates `kill`
    /// itself failing (e.g. the process had already exited on its own
    /// between the last poll and this call) — `wait` still always runs,
    /// so the process is reaped either way.
    fn kill_and_reap(&mut self) -> io::Result<()>;
}

#[cfg(target_os = "macos")]
impl Waitable for std::process::Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn kill_and_reap(&mut self) -> io::Result<()> {
        let _ = std::process::Child::kill(self);
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
    on_outcome: AsyncOutcomeHandler,
) {
    thread::spawn(move || {
        watch_and_report(
            waitable,
            deadline,
            poll_interval,
            &dirs,
            &program,
            on_outcome,
        );
    });
}

/// Polls `waitable` every `poll_interval` until it exits or `deadline`
/// elapses (from this call, not from when the process actually
/// started). On a `try_wait` error, or once the deadline is hit, the
/// process is killed and reaped (see [`Waitable::kill_and_reap`]) and
/// the outcome is [`LaunchOutcome::Unconfirmed`] either way — a
/// `try_wait` error means we genuinely don't know what happened, and a
/// deadline kill is us *ending* the process, not it reporting a result,
/// so neither can be treated as a definite failure worth falling back
/// from. Always terminates: either `waitable` exits on its own, or the
/// deadline forces it.
#[cfg(target_os = "macos")]
fn watch<W: Waitable>(
    mut waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
) -> LaunchOutcome {
    let start = std::time::Instant::now();
    loop {
        match waitable.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    LaunchOutcome::Succeeded
                } else {
                    LaunchOutcome::DefiniteFailure
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = waitable.kill_and_reap();
                return LaunchOutcome::Unconfirmed;
            }
        }
        if start.elapsed() >= deadline {
            let _ = waitable.kill_and_reap();
            return LaunchOutcome::Unconfirmed;
        }
        thread::sleep(poll_interval);
    }
}

/// Runs [`watch`], logs what it found, and reports the
/// [`LaunchOutcome`] to `on_outcome` exactly once — unconditionally,
/// including on success, since callers (e.g. a fallback launch) may
/// need to log a confirmed-opened line that `watch_and_report` itself
/// has no context for.
#[cfg(target_os = "macos")]
fn watch_and_report<W: Waitable>(
    waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
    dirs: &Dirs,
    program: &str,
    on_outcome: AsyncOutcomeHandler,
) {
    let outcome = watch(waitable, deadline, poll_interval);
    match outcome {
        LaunchOutcome::Succeeded => {}
        LaunchOutcome::DefiniteFailure => {
            crate::platform::logging::log_line(
                dirs,
                &format!("{program}: confirmed exited with a failing status"),
            );
        }
        LaunchOutcome::Unconfirmed => {
            crate::platform::logging::log_line(
                dirs,
                &format!(
                    "{program}: could not confirm success (a try_wait error, or still running \
                     past the {deadline:?} deadline, now killed)"
                ),
            );
        }
    }
    on_outcome(outcome);
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
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

    fn record_outcome(slot: Arc<std::sync::Mutex<Vec<LaunchOutcome>>>) -> AsyncOutcomeHandler {
        Box::new(move |outcome| slot.lock().unwrap().push(outcome))
    }

    /// Reports exactly the scripted sequence of poll results (an `Err`
    /// step simulates `try_wait` itself failing), then keeps reporting
    /// "still running" forever (so a deadline test can run it past its
    /// deadline).
    enum Step {
        StillRunning,
        Exited(ExitStatus),
        TryWaitErrors,
    }

    struct ScriptedWaitable {
        remaining: std::collections::VecDeque<Step>,
        kill_called: Arc<AtomicBool>,
        /// Whether `kill_and_reap` still calls through (reaps) even
        /// when asked to simulate `kill` itself failing — mirrors the
        /// real `Child` impl's "ignore kill's error, always wait"
        /// contract, so this fake can stand in for it in `watch`-level
        /// tests without spawning a real process.
        reaped: Arc<AtomicBool>,
    }

    impl Waitable for ScriptedWaitable {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            match self.remaining.pop_front() {
                Some(Step::StillRunning) | None => Ok(None),
                Some(Step::Exited(status)) => Ok(Some(status)),
                Some(Step::TryWaitErrors) => Err(io::Error::other("try_wait failed")),
            }
        }

        fn kill_and_reap(&mut self) -> io::Result<()> {
            self.kill_called.store(true, Ordering::SeqCst);
            // Real `kill` failing (e.g. already exited) must not skip
            // reaping — see the real impl's doc comment.
            self.reaped.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn waitable(steps: Vec<Step>) -> ScriptedWaitable {
        ScriptedWaitable {
            remaining: std::collections::VecDeque::from(steps),
            kill_called: Arc::new(AtomicBool::new(false)),
            reaped: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn zero_exit_reports_succeeded_and_never_kills() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let w = waitable(vec![Step::Exited(exit_status(0))]);
        let kill_called = w.kill_called.clone();
        watch_and_report(
            w,
            Duration::from_millis(200),
            Duration::from_millis(5),
            &test_dirs(),
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Succeeded]);
        assert!(!kill_called.load(Ordering::SeqCst));
    }

    #[test]
    fn non_zero_exit_reports_definite_failure_exactly_once() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        // Raw wait-status 256 => exit code 1.
        let w = waitable(vec![Step::Exited(exit_status(256))]);
        watch_and_report(
            w,
            Duration::from_millis(200),
            Duration::from_millis(5),
            &test_dirs(),
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert_eq!(
            *outcomes.lock().unwrap(),
            vec![LaunchOutcome::DefiniteFailure]
        );
    }

    #[test]
    fn deadline_kills_reaps_and_reports_unconfirmed_exactly_once() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let w = waitable(vec![]); // always "still running"
        let kill_called = w.kill_called.clone();
        let reaped = w.reaped.clone();
        watch_and_report(
            w,
            Duration::from_millis(60),
            Duration::from_millis(10),
            &test_dirs(),
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert!(
            kill_called.load(Ordering::SeqCst),
            "must kill a hung process"
        );
        assert!(reaped.load(Ordering::SeqCst), "must reap after killing");
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Unconfirmed]);
    }

    #[test]
    fn try_wait_error_kills_reaps_and_reports_unconfirmed_exactly_once() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let w = waitable(vec![Step::TryWaitErrors]);
        let kill_called = w.kill_called.clone();
        let reaped = w.reaped.clone();
        watch_and_report(
            w,
            Duration::from_secs(5),
            Duration::from_millis(10),
            &test_dirs(),
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert!(
            kill_called.load(Ordering::SeqCst),
            "a try_wait error must still attempt to kill, not just give up"
        );
        assert!(reaped.load(Ordering::SeqCst), "must still reap");
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Unconfirmed]);
    }

    /// Exercises the *real* `impl Waitable for Child`'s `kill_and_reap`
    /// directly: spawns a real process that exits on its own almost
    /// immediately, waits for that to happen, then calls
    /// `kill_and_reap` — at which point the real `kill(2)` syscall is
    /// expected to fail (nothing left to signal), and this must still
    /// return `Ok(())` because `wait` always runs regardless.
    #[test]
    fn real_child_kill_and_reap_tolerates_kill_failing_on_an_already_exited_process() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        // Give it a moment to actually exit before we try to kill it.
        for _ in 0..50 {
            if matches!(Waitable::try_wait(&mut child), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            Waitable::kill_and_reap(&mut child).is_ok(),
            "kill_and_reap must still succeed (and reap) even when kill() itself fails"
        );
    }

    /// End-to-end with a real short-lived child that exits non-zero:
    /// proves the whole spawn -> watch -> report pipeline reaps it and
    /// reports a definite failure, without any scripted fake involved.
    #[test]
    fn real_child_with_nonzero_exit_is_reaped_and_reports_definite_failure() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        watch_and_report(
            child,
            Duration::from_secs(5),
            Duration::from_millis(10),
            &test_dirs(),
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert_eq!(
            *outcomes.lock().unwrap(),
            vec![LaunchOutcome::DefiniteFailure]
        );
    }

    /// `try_wait` blocks on a two-party barrier until released. Runs
    /// `spawn_watcher` on a helper thread and waits on a channel (bounded
    /// to a generous 5s) for it to return: if `spawn_watcher` ever
    /// regressed into waiting for the watcher thread (rather than only
    /// spawning it), the helper thread would never reach the `tx.send`
    /// below, and this test fails on the `recv_timeout` instead of
    /// hanging forever.
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
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            spawn_watcher(
                waitable,
                Duration::from_secs(5),
                Duration::from_millis(10),
                test_dirs(),
                "fake-open".to_string(),
                Box::new(|_outcome| {}),
            );
            // Only reached if `spawn_watcher` didn't block on the
            // watcher thread's `try_wait`.
            let _ = tx.send(());
            // Releases the still-blocked watcher thread so it (and this
            // helper thread) can exit cleanly.
            barrier.wait();
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("spawn_watcher must return promptly, without waiting for the child");
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
