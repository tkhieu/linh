//! Desktop notifications via `osascript`(1) (built into macOS), the same
//! "shell out, fall back to logging on failure" shape Linux uses for
//! `notify-send`. `summary`/`body` are passed as separate process
//! arguments (`argv`), read by the script via `on run argv`, rather than
//! interpolated into the AppleScript source — they never run as, or
//! break out into, AppleScript code.

use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::logging::log_line;

use super::dirs::Dirs;

const OSASCRIPT: &str = "/usr/bin/osascript";
const DISPLAY_NOTIFICATION_SCRIPT: &str =
    "on run argv\n  display notification (item 2 of argv) with title (item 1 of argv)\nend run";

/// How long `osascript` gets to display the notification before Linh
/// gives up on it — `notify_error` runs on whichever thread hit the
/// notification-worthy condition (routing, startup), none of which may
/// stall indefinitely on a wedged AppleScript helper.
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often that wait polls for `osascript`'s exit.
const OSASCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn notify_error(dirs: &Dirs, summary: &str, body: &str) {
    let sent = Command::new(OSASCRIPT)
        .arg("-e")
        .arg(DISPLAY_NOTIFICATION_SCRIPT)
        .arg("--")
        .arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|child| run_bounded(child, OSASCRIPT_TIMEOUT, OSASCRIPT_POLL_INTERVAL))
        .unwrap_or(false);

    if !sent {
        log_line(dirs, &format!("notification failed: {summary}: {body}"));
    }
}

/// The subset of [`Child`] needed to bound how long `notify_error` waits
/// for it, so tests can inject a fake instead of a real `osascript`
/// process.
trait Waitable {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Kills and waits for the process to exit, tolerating `kill`
    /// itself failing (e.g. it had already exited on its own).
    fn kill_and_wait(&mut self) -> io::Result<ExitStatus>;
}

impl Waitable for Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn kill_and_wait(&mut self) -> io::Result<ExitStatus> {
        let _ = Child::kill(self);
        Child::wait(self)
    }
}

/// Polls `waitable` for up to `timeout`, sleeping `poll_interval`
/// between checks, instead of one unbounded blocking `Command::status()`
/// call — a wedged or merely slow `osascript` must never stall the
/// caller. Past the timeout the process is killed; unlike
/// `platform::launcher`'s process watcher (which guards against exactly
/// that for long-running browsers/apps), the final `wait` here is left
/// unbounded, since `osascript` displaying one notification is a
/// short-lived helper with nothing to be wedged in.
fn run_bounded<W: Waitable>(mut waitable: W, timeout: Duration, poll_interval: Duration) -> bool {
    let start = Instant::now();
    loop {
        match waitable.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {}
            Err(_) => return false,
        }
        if start.elapsed() >= timeout {
            return waitable
                .kill_and_wait()
                .map(|status| status.success())
                .unwrap_or(false);
        }
        thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn exit_status(code: i32) -> ExitStatus {
        ExitStatusExt::from_raw(code)
    }

    /// Reports `None` (still running) for every step before finally
    /// reporting `exited`, if any — an empty `steps` means "never
    /// exits," so a timeout test can run it past its bound.
    struct Scripted {
        steps: VecDeque<Option<ExitStatus>>,
        killed: Arc<AtomicBool>,
    }

    impl Waitable for Scripted {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(self.steps.pop_front().flatten())
        }

        fn kill_and_wait(&mut self) -> io::Result<ExitStatus> {
            self.killed.store(true, Ordering::SeqCst);
            // A killed process' wait status reports a failure.
            Ok(exit_status(256))
        }
    }

    fn scripted(steps: Vec<Option<ExitStatus>>) -> (Scripted, Arc<AtomicBool>) {
        let killed = Arc::new(AtomicBool::new(false));
        (
            Scripted {
                steps: VecDeque::from(steps),
                killed: killed.clone(),
            },
            killed,
        )
    }

    #[test]
    fn successful_exit_within_timeout_is_sent_without_killing() {
        let (w, killed) = scripted(vec![Some(exit_status(0))]);
        assert!(run_bounded(
            w,
            Duration::from_millis(200),
            Duration::from_millis(5)
        ));
        assert!(!killed.load(Ordering::SeqCst));
    }

    #[test]
    fn non_zero_exit_is_reported_as_not_sent() {
        let (w, _killed) = scripted(vec![Some(exit_status(256))]);
        assert!(!run_bounded(
            w,
            Duration::from_millis(200),
            Duration::from_millis(5)
        ));
    }

    #[test]
    fn gives_up_and_kills_a_wedged_osascript_instead_of_blocking_forever() {
        let (w, killed) = scripted(vec![]); // never exits
        let sent = run_bounded(w, Duration::from_millis(30), Duration::from_millis(5));
        assert!(!sent, "a killed process must never count as sent");
        assert!(
            killed.load(Ordering::SeqCst),
            "must kill a hung osascript rather than block forever"
        );
    }
}
