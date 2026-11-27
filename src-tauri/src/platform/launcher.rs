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

/// Spawns `cmd` detached from Linh (never through a shell: `program` and
/// `args` are passed directly to `exec`), and reaps it in a background
/// thread so it never lingers as a zombie. Returns as soon as the
/// process has been spawned (`Command::spawn()` succeeding), never
/// waiting for it to exit, on any platform — this must never block the
/// caller, which can be Tauri's
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
        // Fully qualified: on macOS, `Waitable` (same module, consuming
        // `self`) is also in scope and would otherwise be preferred by
        // method-call resolution over this inherent, `&mut self` one.
        let _ = std::process::Child::wait(&mut child);
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

/// The subset of [`std::process::Child`] a bounded wait needs: a
/// non-blocking poll, a non-blocking kill, and a *consuming*, blocking
/// `wait` used only from the detached thread [`kill_and_detach`] spawns
/// — never from the waiting caller's own thread. `Send + 'static` so a
/// still-alive one can be handed to that thread. Lets tests inject a
/// fake instead of a real OS process. Shared by this module's launch
/// watcher and `platform::macos::notify`'s bounded `osascript` wait, so
/// there is exactly one "kill, then reap without blocking the caller"
/// implementation.
#[cfg(target_os = "macos")]
pub(crate) trait Waitable: Send + 'static {
    /// Non-blocking: `Ok(None)` while still running.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Non-blocking. Tolerates failing (e.g. the process had already
    /// exited on its own between the last poll and this call).
    fn kill(&mut self);
    /// Consumes `self`, blocking until it exits.
    fn wait(self) -> io::Result<ExitStatus>;
}

#[cfg(target_os = "macos")]
impl Waitable for std::process::Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn kill(&mut self) {
        let _ = std::process::Child::kill(self);
    }

    fn wait(mut self) -> io::Result<ExitStatus> {
        std::process::Child::wait(&mut self)
    }
}

/// What [`wait_bounded`] found.
#[cfg(target_os = "macos")]
pub(crate) enum BoundedWaitOutcome {
    Exited(ExitStatus),
    /// Not confirmed either way within the bound — a `try_wait` error,
    /// or still running once the bound elapsed. Either way the process
    /// has been killed and handed off to a detached reaper thread (see
    /// [`kill_and_detach`]); nothing further to wait on here.
    GaveUp,
}

/// Polls `waitable` every `poll_interval` until it exits or `bound`
/// elapses (from this call, not from when the process actually
/// started), instead of one unbounded blocking wait. On a `try_wait`
/// error, or once `bound` is hit while still running, the process is
/// killed and handed to [`kill_and_detach`] rather than waited on here —
/// so this always returns within `bound`, whatever `waitable` does.
#[cfg(target_os = "macos")]
pub(crate) fn wait_bounded<W: Waitable>(
    mut waitable: W,
    bound: std::time::Duration,
    poll_interval: std::time::Duration,
) -> BoundedWaitOutcome {
    let start = std::time::Instant::now();
    loop {
        match waitable.try_wait() {
            Ok(Some(status)) => return BoundedWaitOutcome::Exited(status),
            Ok(None) => {}
            Err(_) => {
                kill_and_detach(waitable);
                return BoundedWaitOutcome::GaveUp;
            }
        }
        if start.elapsed() >= bound {
            kill_and_detach(waitable);
            return BoundedWaitOutcome::GaveUp;
        }
        thread::sleep(poll_interval);
    }
}

/// Kills `waitable` and hands it to a new, detached thread that blocks
/// on its (consuming) [`Waitable::wait`] — so it's still reaped
/// whenever it actually exits, without the caller of [`wait_bounded`]
/// ever blocking past its own bound. Trade-off: if `kill` doesn't take
/// (the process is wedged in, say, an uninterruptible syscall), that
/// thread stays parked in `wait` for as long as the process survives —
/// accepted, since only that one isolated thread pays for it, never the
/// caller, and a `kill` that never takes is the rare, pathological case.
#[cfg(target_os = "macos")]
fn kill_and_detach<W: Waitable>(mut waitable: W) {
    waitable.kill();
    thread::spawn(move || {
        let _ = waitable.wait();
    });
}

/// Spawns exactly one worker thread that calls [`watch_and_report`] and
/// then terminates — it never lingers, however `waitable` resolves.
/// Returns immediately; the thread itself does all the waiting.
#[cfg(target_os = "macos")]
fn spawn_watcher<W: Waitable>(
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

/// Waits (bounded by [`wait_bounded`]) for `waitable` to exit and maps
/// that to a [`LaunchOutcome`]: an exit status becomes
/// [`LaunchOutcome::Succeeded`]/[`LaunchOutcome::DefiniteFailure`], and
/// not confirming either way within `deadline` becomes
/// [`LaunchOutcome::Unconfirmed`] — a `try_wait` error means we
/// genuinely don't know what happened, and a deadline kill is us
/// *ending* the process, not it reporting a result, so neither can be
/// treated as a definite failure worth falling back from. Always
/// terminates within `deadline`.
#[cfg(target_os = "macos")]
fn watch<W: Waitable>(
    waitable: W,
    deadline: std::time::Duration,
    poll_interval: std::time::Duration,
) -> LaunchOutcome {
    match wait_bounded(waitable, deadline, poll_interval) {
        BoundedWaitOutcome::Exited(status) => {
            if status.success() {
                LaunchOutcome::Succeeded
            } else {
                LaunchOutcome::DefiniteFailure
            }
        }
        BoundedWaitOutcome::GaveUp => LaunchOutcome::Unconfirmed,
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
    use std::sync::atomic::{AtomicBool, Ordering};
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
        Exited(ExitStatus),
        TryWaitErrors,
    }

    struct ScriptedWaitable {
        remaining: std::collections::VecDeque<Step>,
        kill_called: Arc<AtomicBool>,
        /// Signaled from [`Waitable::wait`], which — once this is
        /// handed to [`kill_and_detach`] — runs on a detached thread,
        /// not synchronously with the caller. Tests must `recv` this
        /// rather than check a flag right after the caller returns.
        reaped_tx: mpsc::Sender<()>,
    }

    impl Waitable for ScriptedWaitable {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            match self.remaining.pop_front() {
                None => Ok(None),
                Some(Step::Exited(status)) => Ok(Some(status)),
                Some(Step::TryWaitErrors) => Err(io::Error::other("try_wait failed")),
            }
        }

        fn kill(&mut self) {
            self.kill_called.store(true, Ordering::SeqCst);
        }

        fn wait(self) -> io::Result<ExitStatus> {
            let _ = self.reaped_tx.send(());
            Ok(exit_status(0))
        }
    }

    fn waitable(steps: Vec<Step>) -> (ScriptedWaitable, Arc<AtomicBool>, mpsc::Receiver<()>) {
        let kill_called = Arc::new(AtomicBool::new(false));
        let (reaped_tx, reaped_rx) = mpsc::channel();
        (
            ScriptedWaitable {
                remaining: std::collections::VecDeque::from(steps),
                kill_called: kill_called.clone(),
                reaped_tx,
            },
            kill_called,
            reaped_rx,
        )
    }

    /// `recv`s with a generous bound, rather than asserting a flag right
    /// after the caller returns — [`kill_and_detach`] reaps on its own
    /// thread, asynchronously with `watch`/`watch_and_report` returning.
    fn expect_reaped(reaped_rx: &mpsc::Receiver<()>) {
        reaped_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the detached reaper must still call wait()");
    }

    // --- `wait_bounded`: tested directly with a short bound instead of
    // the real 5s/10s constants the real call sites use, so this never
    // sleeps more than a few tens of milliseconds. ---

    #[test]
    fn wait_bounded_reports_exited_as_soon_as_try_wait_does() {
        let (w, kill_called, _reaped_rx) = waitable(vec![Step::Exited(exit_status(0))]);
        match wait_bounded(w, Duration::from_secs(5), Duration::from_millis(5)) {
            BoundedWaitOutcome::Exited(status) => assert!(status.success()),
            BoundedWaitOutcome::GaveUp => panic!("expected Exited"),
        }
        assert!(
            !kill_called.load(Ordering::SeqCst),
            "never ran, never killed"
        );
    }

    #[test]
    fn wait_bounded_gives_up_kills_and_detaches_rather_than_blocking_forever() {
        let (w, kill_called, reaped_rx) = waitable(vec![]); // always "still running"
        let outcome = wait_bounded(w, Duration::from_millis(30), Duration::from_millis(5));
        assert!(matches!(outcome, BoundedWaitOutcome::GaveUp));
        assert!(
            kill_called.load(Ordering::SeqCst),
            "must kill once the bound elapses, not hang"
        );
        expect_reaped(&reaped_rx);
    }

    #[test]
    fn wait_bounded_kills_and_detaches_on_a_try_wait_error_too() {
        let (w, kill_called, reaped_rx) = waitable(vec![Step::TryWaitErrors]);
        let outcome = wait_bounded(w, Duration::from_secs(5), Duration::from_millis(5));
        assert!(matches!(outcome, BoundedWaitOutcome::GaveUp));
        assert!(
            kill_called.load(Ordering::SeqCst),
            "a try_wait error must still attempt to kill, not just give up silently"
        );
        expect_reaped(&reaped_rx);
    }

    /// Never exits, and blocks in `wait` until explicitly released —
    /// stands in for a `kill(2)` that doesn't take right away. Proves
    /// that [`watch_and_report`] (and so the routing worker calling it)
    /// returns within its own deadline regardless, while the detached
    /// reaper this hands off to keeps its own, separate hold on the
    /// still-alive child until released.
    struct SlowToReapWaitable {
        release: Arc<Barrier>,
        reaped: Arc<AtomicBool>,
    }

    impl Waitable for SlowToReapWaitable {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(None)
        }

        fn kill(&mut self) {}

        fn wait(self) -> io::Result<ExitStatus> {
            self.release.wait();
            self.reaped.store(true, Ordering::SeqCst);
            Ok(exit_status(0))
        }
    }

    #[test]
    fn routing_worker_returns_promptly_while_a_slow_reaper_still_owns_the_child() {
        let release = Arc::new(Barrier::new(2));
        let reaped = Arc::new(AtomicBool::new(false));
        let w = SlowToReapWaitable {
            release: release.clone(),
            reaped: reaped.clone(),
        };
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dirs = test_dirs();

        let started = std::time::Instant::now();
        watch_and_report(
            w,
            Duration::from_millis(30),
            Duration::from_millis(5),
            &dirs,
            "fake-open",
            record_outcome(outcomes.clone()),
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "must not block on the detached reaper"
        );
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Unconfirmed]);
        assert!(
            !reaped.load(Ordering::SeqCst),
            "the reaper must still be blocked on its own thread, separate from the caller"
        );

        // Release the detached thread so it (and this test) can exit
        // cleanly, and confirm it does go on to reap the child.
        release.wait();
        for _ in 0..200 {
            if reaped.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            reaped.load(Ordering::SeqCst),
            "the detached thread must eventually reap it"
        );
    }

    #[test]
    fn zero_exit_reports_succeeded_and_never_kills() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (w, kill_called, _reaped_rx) = waitable(vec![Step::Exited(exit_status(0))]);
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
        let (w, _kill_called, _reaped_rx) = waitable(vec![Step::Exited(exit_status(256))]);
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
    fn deadline_kills_detaches_a_reaper_and_reports_unconfirmed_exactly_once() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (w, kill_called, reaped_rx) = waitable(vec![]); // always "still running"
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
        expect_reaped(&reaped_rx);
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Unconfirmed]);
    }

    #[test]
    fn try_wait_error_kills_detaches_a_reaper_and_reports_unconfirmed_exactly_once() {
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (w, kill_called, reaped_rx) = waitable(vec![Step::TryWaitErrors]);
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
        expect_reaped(&reaped_rx);
        assert_eq!(*outcomes.lock().unwrap(), vec![LaunchOutcome::Unconfirmed]);
    }

    /// Exercises the *real* `impl Waitable for Child`'s `kill`/`wait`
    /// directly: spawns a real process that exits on its own almost
    /// immediately, waits for that to happen, then calls `kill` (whose
    /// underlying `kill(2)` is expected to fail — nothing left to
    /// signal) followed by `wait`, which must still succeed and reap it
    /// regardless of `kill`'s own result.
    #[test]
    fn real_child_kill_tolerates_failing_on_an_already_exited_process() {
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
        Waitable::kill(&mut child);
        assert!(
            Waitable::wait(child).is_ok(),
            "wait must still succeed (and reap) even when kill() itself failed"
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

        fn kill(&mut self) {}

        fn wait(self) -> io::Result<ExitStatus> {
            Ok(exit_status(0))
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
