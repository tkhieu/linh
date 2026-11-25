//! Opt-in, OFF-by-default instrumentation for the cold WebKit webview
//! creation cost (`LINH_PROFILE_WEBVIEW`), plus an unconditional, cheap
//! log line for how long `prompt` (config-declared, created at startup)
//! and `settings` (created lazily on first demand, see
//! [`crate::settings_window`]) each take to finish their first page
//! load. Both exist only to gather numbers for tkhieu/linh#15; neither
//! changes default runtime behavior when the env var is unset.

#[cfg(target_os = "linux")]
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::time::Instant;

use tauri::webview::PageLoadEvent;
use tauri::AppHandle;
#[cfg(target_os = "linux")]
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::platform::current::dirs::Dirs;
use crate::platform::logging;

const ENV_VAR: &str = "LINH_PROFILE_WEBVIEW";
const MIN_SAMPLES: u32 = 1;
const MAX_SAMPLES: u32 = 50;
#[cfg(target_os = "linux")]
const BUILD_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "linux")]
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(target_os = "linux")]
const DESTROYED_EVENT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_os = "linux")]
const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Parses `LINH_PROFILE_WEBVIEW`'s value into a validated sample count
/// (1..=50). `None` for unset, non-numeric, or out-of-range values —
/// the caller treats all of those as "feature disabled".
pub fn parse_sample_count(value: Option<&str>) -> Option<u32> {
    let n: u32 = value?.trim().parse().ok()?;
    (MIN_SAMPLES..=MAX_SAMPLES).contains(&n).then_some(n)
}

/// min/median/max (in whatever unit `samples` is already in). `None` if
/// `samples` is empty.
pub fn stats(mut samples: Vec<u128>) -> Option<(u128, u128, u128)> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let min = samples[0];
    let max = *samples.last().unwrap();
    let mid = samples.len() / 2;
    let median = if samples.len().is_multiple_of(2) {
        (samples[mid - 1] + samples[mid]) / 2
    } else {
        samples[mid]
    };
    Some((min, median, max))
}

// --- window "ready" logging (unconditional, cheap) ---------------------

/// The instant Settings' creation was first requested (see
/// [`record_settings_create_requested`]), set once and read back when its
/// first page load finishes. `settings` has no config-declared startup
/// slot anymore — it's built on demand — so unlike `prompt` its "ready"
/// line can't be timed from `process_start`; it's timed from this instant
/// instead.
static SETTINGS_CREATE_REQUESTED: OnceLock<Instant> = OnceLock::new();

/// Records that Settings' creation was just requested, so its first
/// page-load-finished event (see [`log_window_ready_if_relevant`]) can be
/// timed from here. Idempotent — only the first call (Settings is only
/// ever ultimately (re)built once per process, since closing it hides
/// rather than destroys it) actually sets the instant.
pub fn record_settings_create_requested(at: Instant) {
    let _ = SETTINGS_CREATE_REQUESTED.set(at);
}

/// Whether `prompt`'s and `settings`' ready lines have already been
/// logged (each must fire at most once). Updated and read synchronously
/// from inside the global `on_page_load` hook itself — no background
/// thread or timer, so with `LINH_PROFILE_WEBVIEW` unset this adds
/// nothing beyond a mutex lock per page-load event.
#[derive(Default)]
struct ReadyLogged {
    prompt: bool,
    settings: bool,
}

fn ready_logged_state() -> &'static Mutex<ReadyLogged> {
    static STATE: OnceLock<Mutex<ReadyLogged>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ReadyLogged::default()))
}

/// The ready line for `label` timed at `ms`, if it hasn't already been
/// logged (marking it logged as a side effect) — `None` for an unknown
/// label or a repeat event. Pure state transition, directly
/// unit-testable without any Tauri or timing types.
fn record_ready_and_take_line(state: &mut ReadyLogged, label: &str, ms: u128) -> Option<String> {
    match label {
        "prompt" => {
            if state.prompt {
                return None;
            }
            state.prompt = true;
            Some(format!(
                "webview-create startup ready (process_start->page_load_finished): prompt={ms}ms"
            ))
        }
        "settings" => {
            if state.settings {
                return None;
            }
            state.settings = true;
            Some(format!(
                "webview-create settings first-open ready (create_requested->page_load_finished): settings={ms}ms"
            ))
        }
        _ => None,
    }
}

/// Unconditional, cheap: logs, exactly once each, how long `prompt`
/// (timed from `process_start`) and `settings` (timed from
/// [`record_settings_create_requested`], since it has no startup slot to
/// time from) took to finish their first page load. Tauri has no hook
/// that fires on a window's *creation* specifically — `Builder::
/// on_page_load` is the cheapest signal it does expose, so this reports
/// "ready" (page load finished) rather than "created". Call from a
/// global `on_page_load` hook for every webview; it ignores anything
/// that isn't `settings` or `prompt`, and ignores `settings` entirely if
/// its creation was never requested (can't happen in practice, since
/// `settings_window::create` always calls
/// [`record_settings_create_requested`] before building the window).
pub fn log_window_ready_if_relevant(
    dirs: &Dirs,
    process_start: Instant,
    label: &str,
    event: PageLoadEvent,
) {
    if event != PageLoadEvent::Finished {
        return;
    }
    let ms = match label {
        "prompt" => process_start.elapsed().as_millis(),
        "settings" => match SETTINGS_CREATE_REQUESTED.get() {
            Some(requested_at) => requested_at.elapsed().as_millis(),
            None => return,
        },
        _ => return,
    };
    let line = {
        let mut state = ready_logged_state().lock().unwrap();
        record_ready_and_take_line(&mut state, label, ms)
    };
    if let Some(line) = line {
        logging::log_line(dirs, &line);
    }
}

// --- opt-in cold webview-create profiling (Linux only: it measures the
// WebKitGTK-specific `WebKitWebProcess`) -------------------------------

/// If `LINH_PROFILE_WEBVIEW=N` is set to a valid sample count, spawns a
/// background thread that creates, measures and destroys N throwaway
/// webviews loading `prompt.html`, logging each sample plus
/// min/median/max to the normal Linh log. Does nothing otherwise (no
/// extra windows, no extra work, no extra logging).
#[cfg(target_os = "linux")]
pub fn spawn_if_enabled(app: &AppHandle, dirs: Dirs) {
    let Some(count) = parse_sample_count(std::env::var(ENV_VAR).ok().as_deref()) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || run_profile(&app, &dirs, count));
}

// ported in a later change: this profiling measures WebKitGTK's
// `WebKitWebProcess` specifically, so it has nothing to do on other
// OSes. Logs instead of silently doing nothing, so setting the env var
// there isn't mistaken for the feature having run.
#[cfg(not(target_os = "linux"))]
pub fn spawn_if_enabled(_app: &AppHandle, dirs: Dirs) {
    if parse_sample_count(std::env::var(ENV_VAR).ok().as_deref()).is_some() {
        logging::log_line(
            &dirs,
            &format!("{ENV_VAR} is set, but webview-create profiling is Linux-only; ignoring"),
        );
    }
}

#[cfg(target_os = "linux")]
fn run_profile(app: &AppHandle, dirs: &Dirs, count: u32) {
    logging::log_line(
        dirs,
        &format!("webview-create profiling {count} sample(s) requested via {ENV_VAR}"),
    );

    let mut build_ms_samples = Vec::with_capacity(count as usize);
    let mut ready_ms_samples = Vec::with_capacity(count as usize);
    let mut attributed_total: u32 = 0;
    let mut attributed_exited: u32 = 0;

    for index in 0..count {
        match run_one_sample(app, dirs, index) {
            Ok(outcome) => {
                logging::log_line(
                    dirs,
                    &format!(
                        "webview-create sample {}/{count} build={}ms ready={}ms",
                        index + 1,
                        outcome.build_ms,
                        outcome.ready_ms
                    ),
                );
                build_ms_samples.push(outcome.build_ms);
                ready_ms_samples.push(outcome.ready_ms);
                if outcome.web_process_pid.is_some() {
                    attributed_total += 1;
                    if outcome.process_exit_ms.is_some() {
                        attributed_exited += 1;
                    }
                }
            }
            Err(err) => {
                logging::log_line(
                    dirs,
                    &format!(
                        "webview-create sample {}/{count} FAILED (excluded from stats): {err}",
                        index + 1
                    ),
                );
            }
        }
    }

    log_stats_line(dirs, "build", &build_ms_samples);
    log_stats_line(dirs, "ready (page load finished)", &ready_ms_samples);
    logging::log_line(
        dirs,
        &format!(
            "webview-create WebKitWebProcess teardown: {attributed_exited}/{attributed_total} attributable sample(s) exited within {}s",
            PROCESS_EXIT_TIMEOUT.as_secs()
        ),
    );
}

#[cfg(target_os = "linux")]
fn log_stats_line(dirs: &Dirs, label: &str, samples: &[u128]) {
    match stats(samples.to_vec()) {
        Some((min, median, max)) => logging::log_line(
            dirs,
            &format!(
                "webview-create {label} stats: min={min}ms median={median}ms max={max}ms (n={})",
                samples.len()
            ),
        ),
        None => logging::log_line(
            dirs,
            &format!("webview-create {label} stats: no samples collected"),
        ),
    }
}

#[cfg(target_os = "linux")]
struct SampleOutcome {
    build_ms: u128,
    ready_ms: u128,
    /// This sample's own `WebKitWebProcess` pid, if it could be
    /// unambiguously attributed (see [`identify_new_webkit_pid`]).
    web_process_pid: Option<u32>,
    /// Milliseconds from destroy-scheduled to that pid actually exiting,
    /// or `None` if it was still running at [`PROCESS_EXIT_TIMEOUT`], or
    /// if `web_process_pid` itself was never known.
    process_exit_ms: Option<u128>,
}

/// Creates one throwaway, hidden `WebviewWindow` loading `prompt.html`,
/// times `WebviewWindowBuilder::build()` and the wall time until its
/// first `PageLoadEvent::Finished`, then always destroys it via
/// [`destroy_and_wait`] — on every exit path, including a build or
/// page-load timeout, so a throwaway window is never leaked even when a
/// sample fails. A sample that times out is `Err` (excluded from the
/// min/median/max stats by the caller), not a zeroed/guessed value.
///
/// Built hidden (`visible(false)`), matching exactly how the real
/// `settings`/`prompt` windows are created — not shown off-screen, since
/// GNOME Wayland toplevels ignore a client-set absolute position, which
/// would risk the throwaway window briefly flashing on screen instead.
/// Linh's own idle baseline already confirms a hidden window's
/// `WebKitWebProcess` is spawned without ever showing it, so hidden-only
/// still measures the real creation cost.
#[cfg(target_os = "linux")]
fn run_one_sample(app: &AppHandle, dirs: &Dirs, index: u32) -> Result<SampleOutcome, String> {
    let label = format!("linh-mem-profile-{index}");
    let root_pid = std::process::id();
    let webkit_before = descendant_webkit_pids(root_pid);

    let (build_tx, build_rx) = mpsc::channel::<Result<u128, String>>();
    let (ready_tx, ready_rx) = mpsc::channel::<Instant>();
    let (destroyed_tx, destroyed_rx) = mpsc::channel::<Instant>();

    let schedule_start = Instant::now();
    let app_for_build = app.clone();
    let label_for_build = label.clone();
    let scheduled = app.run_on_main_thread(move || {
        let build_start = Instant::now();
        let built = WebviewWindowBuilder::new(
            &app_for_build,
            &label_for_build,
            WebviewUrl::App("prompt.html".into()),
        )
        .inner_size(360.0, 400.0)
        .resizable(false)
        .decorations(false)
        .skip_taskbar(true)
        .visible(false)
        .on_page_load(move |_window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = ready_tx.send(Instant::now());
            }
        })
        .build();
        if let Ok(window) = &built {
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed) {
                    let _ = destroyed_tx.send(Instant::now());
                }
            });
        }
        let build_elapsed_ms = build_start.elapsed().as_millis();
        let _ = build_tx.send(built.map(|_| build_elapsed_ms).map_err(|e| e.to_string()));
    });

    // Scoped so `?` inside can bail out early while still always falling
    // through to the unconditional destroy/cleanup step below.
    let attempt: Result<(u128, u128), String> = (|| {
        scheduled.map_err(|e| format!("failed to schedule window build: {e}"))?;
        let build_ms = build_rx
            .recv_timeout(BUILD_TIMEOUT)
            .map_err(|_| "timed out waiting for build() to run".to_string())??;
        let ready_at = ready_rx
            .recv_timeout(PAGE_LOAD_TIMEOUT)
            .map_err(|_| "timed out waiting for page load to finish".to_string())?;
        Ok((
            build_ms,
            ready_at.duration_since(schedule_start).as_millis(),
        ))
    })();

    // Only attributable once the page has actually finished loading: any
    // earlier and the process may not exist yet, any later (e.g. after
    // destroy) and it may already be gone.
    let web_process_pid = if attempt.is_ok() {
        identify_new_webkit_pid(root_pid, &webkit_before, dirs, &label)
    } else {
        None
    };

    let process_exit_ms = destroy_and_wait(app, dirs, &label, destroyed_rx, web_process_pid);

    attempt.map(|(build_ms, ready_ms)| SampleOutcome {
        build_ms,
        ready_ms,
        web_process_pid,
        process_exit_ms,
    })
}

/// The single pid present in `after` but not `before`, or `None` if
/// that's not exactly one (nothing new showed up, or more than one did —
/// either way, not safely attributable to this one sample).
#[cfg(target_os = "linux")]
fn pick_new_pid(before: &HashSet<u32>, after: &HashSet<u32>) -> Option<u32> {
    let mut new_pids: Vec<u32> = after.difference(before).copied().collect();
    if new_pids.len() == 1 {
        new_pids.pop()
    } else {
        None
    }
}

/// Diffs the current `WebKitWebProcess` descendants against
/// `webkit_before` (captured right before this sample's window was
/// created) and logs + returns the single new one — this sample's own
/// process. Logs (and returns `None`) if that's ambiguous.
#[cfg(target_os = "linux")]
fn identify_new_webkit_pid(
    root: u32,
    webkit_before: &HashSet<u32>,
    dirs: &Dirs,
    label: &str,
) -> Option<u32> {
    let after = descendant_webkit_pids(root);
    match pick_new_pid(webkit_before, &after) {
        Some(pid) => {
            logging::log_line(
                dirs,
                &format!("webview-create {label}: WebKitWebProcess pid={pid}"),
            );
            Some(pid)
        }
        None => {
            let new_count = after.difference(webkit_before).count();
            logging::log_line(
                dirs,
                &format!(
                    "webview-create {label}: {new_count} new WebKitWebProcess pid(s) observed — ambiguous, not attributing teardown to a pid"
                ),
            );
            None
        }
    }
}

/// Destroys the throwaway webview `label` via `Window::destroy` (never
/// hide/close), on the main thread — logging the scheduling result, the
/// window lookup, and `destroy()`'s own result — then waits for its
/// `WindowEvent::Destroyed`, and, if `web_process_pid` was unambiguously
/// attributed, polls specifically that pid for exit. Always runs, on
/// every call site/exit path, so a throwaway window is never leaked.
/// Returns the attributed pid's exit latency (`None` if unattributed, or
/// attributed but still running after [`PROCESS_EXIT_TIMEOUT`]).
#[cfg(target_os = "linux")]
fn destroy_and_wait(
    app: &AppHandle,
    dirs: &Dirs,
    label: &str,
    destroyed_rx: mpsc::Receiver<Instant>,
    web_process_pid: Option<u32>,
) -> Option<u128> {
    let destroy_start = Instant::now();
    let app_for_destroy = app.clone();
    let label_for_destroy = label.to_string();
    let dirs_for_destroy = dirs.clone();
    let scheduled = app.run_on_main_thread(move || {
        let Some(window) = app_for_destroy.get_webview_window(&label_for_destroy) else {
            logging::log_line(
                &dirs_for_destroy,
                &format!("webview-create {label_for_destroy}: destroy skipped — window not found"),
            );
            return;
        };
        match window.destroy() {
            Ok(()) => logging::log_line(
                &dirs_for_destroy,
                &format!("webview-create {label_for_destroy}: destroy() called"),
            ),
            Err(err) => logging::log_line(
                &dirs_for_destroy,
                &format!("webview-create {label_for_destroy}: destroy() failed: {err}"),
            ),
        }
    });
    if let Err(err) = scheduled {
        logging::log_line(
            dirs,
            &format!("webview-create {label}: failed to schedule destroy: {err}"),
        );
        return None;
    }

    let destroyed_event = match destroyed_rx.recv_timeout(DESTROYED_EVENT_TIMEOUT) {
        Ok(at) => format!("{}ms", at.duration_since(destroy_start).as_millis()),
        Err(_) => "timeout".to_string(),
    };

    let process_exit_ms = web_process_pid.and_then(|pid| {
        wait_for_pid_exit(pid, PROCESS_EXIT_TIMEOUT).map(|elapsed| elapsed.as_millis())
    });
    let process_exit = match (web_process_pid, process_exit_ms) {
        (None, _) => "n/a (pid unattributed)".to_string(),
        (Some(_), Some(ms)) => format!("{ms}ms"),
        (Some(_), None) => "still-running".to_string(),
    };

    logging::log_line(
        dirs,
        &format!(
            "webview-create {label}: pid={} destroyed_event={destroyed_event} process_exit={process_exit}",
            web_process_pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
    );

    process_exit_ms
}

#[cfg(target_os = "linux")]
fn wait_for_pid_exit(pid: u32, timeout: Duration) -> Option<Duration> {
    let start = Instant::now();
    loop {
        if !is_webkit_web_process(pid) {
            return Some(start.elapsed());
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// --- /proc process-tree helpers ----------------------------------------

/// `/proc/<pid>/stat`'s ppid field (4th field, after the possibly
/// space-containing `(comm)`), for every numeric entry under `/proc`.
/// Best-effort: unreadable/raced-away processes are simply absent from
/// the map rather than erroring.
#[cfg(target_os = "linux")]
fn parent_pids() -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return map;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let mut fields = stat[close + 1..].split_whitespace();
        let _state = fields.next();
        let Some(ppid) = fields.next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        map.insert(pid, ppid);
    }
    map
}

#[cfg(target_os = "linux")]
fn is_webkit_web_process(pid: u32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n == "WebKitWebProcess"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn is_descendant_of(pid: u32, root: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut current = pid;
    // `parents` is a snapshot of a real process tree, which cannot cycle;
    // the bound just caps how far a lookup walks before giving up.
    for _ in 0..128 {
        match parents.get(&current) {
            Some(&ppid) if ppid == root => return true,
            Some(&ppid) if ppid == current => return false,
            Some(&ppid) => current = ppid,
            None => return false,
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn descendant_webkit_pids(root: u32) -> HashSet<u32> {
    let parents = parent_pids();
    parents
        .keys()
        .copied()
        .filter(|&pid| is_webkit_web_process(pid) && is_descendant_of(pid, root, &parents))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_count_accepts_the_valid_range() {
        assert_eq!(parse_sample_count(Some("1")), Some(1));
        assert_eq!(parse_sample_count(Some("50")), Some(50));
        assert_eq!(parse_sample_count(Some("10")), Some(10));
    }

    #[test]
    fn parse_sample_count_rejects_out_of_range() {
        assert_eq!(parse_sample_count(Some("0")), None);
        assert_eq!(parse_sample_count(Some("51")), None);
    }

    #[test]
    fn parse_sample_count_rejects_unset_or_non_numeric() {
        assert_eq!(parse_sample_count(None), None);
        assert_eq!(parse_sample_count(Some("")), None);
        assert_eq!(parse_sample_count(Some("abc")), None);
        assert_eq!(parse_sample_count(Some("3.5")), None);
    }

    #[test]
    fn parse_sample_count_trims_whitespace() {
        assert_eq!(parse_sample_count(Some(" 5 ")), Some(5));
    }

    #[test]
    fn stats_of_empty_is_none() {
        assert_eq!(stats(Vec::new()), None);
    }

    #[test]
    fn stats_of_single_sample() {
        assert_eq!(stats(vec![42]), Some((42, 42, 42)));
    }

    #[test]
    fn stats_odd_count_median_is_middle_element() {
        assert_eq!(stats(vec![5, 1, 3]), Some((1, 3, 5)));
    }

    #[test]
    fn stats_even_count_median_is_average_of_middle_two() {
        assert_eq!(stats(vec![10, 20, 30, 40]), Some((10, 25, 40)));
    }

    #[test]
    fn stats_does_not_depend_on_input_order() {
        assert_eq!(stats(vec![3, 1, 2]), stats(vec![1, 2, 3]));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn is_descendant_of_walks_up_the_tree() {
        let mut parents = HashMap::new();
        parents.insert(3u32, 2u32);
        parents.insert(2u32, 1u32);
        assert!(is_descendant_of(3, 1, &parents));
        assert!(!is_descendant_of(3, 99, &parents));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn is_descendant_of_unknown_pid_is_false() {
        let parents = HashMap::new();
        assert!(!is_descendant_of(3, 1, &parents));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn pick_new_pid_exactly_one_new_is_attributed() {
        let before: HashSet<u32> = [1, 2].into_iter().collect();
        let after: HashSet<u32> = [1, 2, 3].into_iter().collect();
        assert_eq!(pick_new_pid(&before, &after), Some(3));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn pick_new_pid_none_new_is_ambiguous() {
        let before: HashSet<u32> = [1, 2].into_iter().collect();
        let after: HashSet<u32> = [1, 2].into_iter().collect();
        assert_eq!(pick_new_pid(&before, &after), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn pick_new_pid_more_than_one_new_is_ambiguous() {
        let before: HashSet<u32> = [1].into_iter().collect();
        let after: HashSet<u32> = [1, 2, 3].into_iter().collect();
        assert_eq!(pick_new_pid(&before, &after), None);
    }

    #[test]
    fn prompt_logs_on_first_event() {
        let mut state = ReadyLogged::default();
        let line = record_ready_and_take_line(&mut state, "prompt", 20);
        assert_eq!(
            line,
            Some(
                "webview-create startup ready (process_start->page_load_finished): prompt=20ms"
                    .to_string()
            )
        );
        assert!(state.prompt);
    }

    #[test]
    fn settings_logs_on_first_event() {
        let mut state = ReadyLogged::default();
        let line = record_ready_and_take_line(&mut state, "settings", 42);
        assert_eq!(
            line,
            Some(
                "webview-create settings first-open ready (create_requested->page_load_finished): settings=42ms"
                    .to_string()
            )
        );
        assert!(state.settings);
    }

    #[test]
    fn each_label_logs_at_most_once_independently_of_the_other() {
        let mut state = ReadyLogged::default();
        assert!(record_ready_and_take_line(&mut state, "prompt", 10).is_some());
        // A repeat "prompt" event must never log again...
        assert_eq!(record_ready_and_take_line(&mut state, "prompt", 99), None);
        // ...but "settings" is independent and must still log its own
        // first event.
        assert!(record_ready_and_take_line(&mut state, "settings", 50).is_some());
        assert_eq!(
            record_ready_and_take_line(&mut state, "settings", 999),
            None
        );
    }

    #[test]
    fn unknown_label_never_logs() {
        let mut state = ReadyLogged::default();
        assert_eq!(record_ready_and_take_line(&mut state, "other", 10), None);
        assert!(!state.prompt);
        assert!(!state.settings);
    }
}
