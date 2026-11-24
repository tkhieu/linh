//! Opt-in, OFF-by-default instrumentation for the cold WebKit webview
//! creation cost (`LINH_PROFILE_WEBVIEW`), plus an unconditional, cheap
//! log line for how long the config-declared `settings`/`prompt`
//! webviews take to finish their first page load. Both exist only to
//! gather numbers for tkhieu/linh#15; neither changes default runtime
//! behavior when the env var is unset.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::platform::linux::dirs::Dirs;
use crate::platform::linux::logging;

const ENV_VAR: &str = "LINH_PROFILE_WEBVIEW";
const MIN_SAMPLES: u32 = 1;
const MAX_SAMPLES: u32 = 50;
const BUILD_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const WEBKIT_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Tracks which startup windows' "ready" line has already been logged,
/// so a window that reloads later (it shouldn't, but defensively) never
/// double-logs. Separated from the Tauri wiring so it's a plain,
/// testable set operation.
fn mark_startup_logged_once(logged: &mut HashSet<String>, label: &str) -> bool {
    logged.insert(label.to_string())
}

fn startup_logged_registry() -> &'static Mutex<HashSet<String>> {
    static REGISTRY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Unconditional, cheap: logs how long each of the `settings`/`prompt`
/// config-declared webviews took (from process start) to finish their
/// first page load. Tauri has no hook that fires on a config-declared
/// window's *creation* specifically (they're built before `setup` runs,
/// outside any closure we control) — `Builder::on_page_load` is the
/// cheapest signal it does expose, so this reports "ready" (page load
/// finished) rather than "created". Call from a global `on_page_load`
/// hook for every webview; it ignores anything that isn't `settings` or
/// `prompt`, and logs at most once per label.
pub fn log_startup_window_ready_if_relevant(
    dirs: &Dirs,
    process_start: Instant,
    label: &str,
    event: PageLoadEvent,
) {
    if event != PageLoadEvent::Finished {
        return;
    }
    if label != "settings" && label != "prompt" {
        return;
    }
    let mut logged = startup_logged_registry().lock().unwrap();
    if !mark_startup_logged_once(&mut logged, label) {
        return;
    }
    drop(logged);
    logging::log_line(
        dirs,
        &format!(
            "webview-create startup {label} ready (process_start->page_load_finished {}ms)",
            process_start.elapsed().as_millis()
        ),
    );
}

/// If `LINH_PROFILE_WEBVIEW=N` is set to a valid sample count, spawns a
/// background thread that creates, measures and destroys N throwaway
/// webviews loading `prompt.html`, logging each sample plus
/// min/median/max to the normal Linh log. Does nothing otherwise (no
/// extra windows, no extra work, no extra logging).
pub fn spawn_if_enabled(app: &AppHandle, dirs: Dirs) {
    let Some(count) = parse_sample_count(std::env::var(ENV_VAR).ok().as_deref()) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || run_profile(&app, &dirs, count));
}

fn run_profile(app: &AppHandle, dirs: &Dirs, count: u32) {
    logging::log_line(
        dirs,
        &format!("webview-create profiling {count} sample(s) requested via {ENV_VAR}"),
    );

    let mut build_ms_samples = Vec::with_capacity(count as usize);
    let mut ready_ms_samples = Vec::with_capacity(count as usize);

    for index in 0..count {
        match run_one_sample(app, dirs, index) {
            Ok(sample) => {
                logging::log_line(
                    dirs,
                    &format!(
                        "webview-create sample {}/{count} build={}ms ready={}ms",
                        index + 1,
                        sample.build_ms,
                        sample.ready_ms
                    ),
                );
                build_ms_samples.push(sample.build_ms);
                ready_ms_samples.push(sample.ready_ms);
            }
            Err(err) => {
                logging::log_line(
                    dirs,
                    &format!("webview-create sample {}/{count} failed: {err}", index + 1),
                );
            }
        }
    }

    log_stats_line(dirs, "build", &build_ms_samples);
    log_stats_line(dirs, "ready (page load finished)", &ready_ms_samples);
}

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

struct Sample {
    build_ms: u128,
    ready_ms: u128,
}

/// Creates one throwaway, hidden `WebviewWindow` loading `prompt.html`,
/// times `WebviewWindowBuilder::build()` and the wall time until its
/// first `PageLoadEvent::Finished`, destroys it (`Window::destroy`, not
/// hide/close), and waits for its `WebKitWebProcess` to actually exit
/// (best-effort; a timeout here is logged by the caller as part of the
/// next sample's numbers looking off, not treated as fatal).
///
/// Built hidden (`visible(false)`), matching exactly how the real
/// `settings`/`prompt` windows are created — not shown off-screen, since
/// GNOME Wayland toplevels ignore a client-set absolute position, which
/// would risk the throwaway window briefly flashing on screen instead.
/// Linh's own idle baseline already confirms a hidden window's
/// `WebKitWebProcess` is spawned without ever showing it, so hidden-only
/// still measures the real creation cost.
fn run_one_sample(app: &AppHandle, dirs: &Dirs, index: u32) -> Result<Sample, String> {
    let label = format!("linh-mem-profile-{index}");
    let root_pid = std::process::id();
    let baseline_webkit = count_descendant_webkit_processes(root_pid);

    let (build_tx, build_rx) = mpsc::channel::<Result<u128, String>>();
    let (ready_tx, ready_rx) = mpsc::channel::<Instant>();

    let schedule_start = Instant::now();
    let app_for_build = app.clone();
    let label_for_build = label.clone();
    app.run_on_main_thread(move || {
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
        let build_elapsed_ms = build_start.elapsed().as_millis();
        let _ = build_tx.send(built.map(|_| build_elapsed_ms).map_err(|e| e.to_string()));
    })
    .map_err(|e| format!("failed to schedule window build: {e}"))?;

    let build_ms = build_rx
        .recv_timeout(BUILD_TIMEOUT)
        .map_err(|_| "timed out waiting for build() to run".to_string())??;

    let ready_at = ready_rx
        .recv_timeout(PAGE_LOAD_TIMEOUT)
        .map_err(|_| "timed out waiting for page load to finish".to_string())?;
    let ready_ms = ready_at.duration_since(schedule_start).as_millis();

    let app_for_destroy = app.clone();
    let label_for_destroy = label;
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = app_for_destroy.get_webview_window(&label_for_destroy) {
            let _ = window.destroy();
        }
    });

    if !wait_for_webkit_process_count_at_most(baseline_webkit, root_pid, WEBKIT_TEARDOWN_TIMEOUT) {
        logging::log_line(
            dirs,
            "webview-create: WebKitWebProcess teardown did not settle within timeout",
        );
    }

    Ok(Sample { build_ms, ready_ms })
}

/// `/proc/<pid>/stat`'s ppid field (4th field, after the possibly
/// space-containing `(comm)`), for every numeric entry under `/proc`.
/// Best-effort: unreadable/raced-away processes are simply absent from
/// the map rather than erroring.
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

fn is_webkit_web_process(pid: u32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n == "WebKitWebProcess"))
        .unwrap_or(false)
}

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

fn count_descendant_webkit_processes(root: u32) -> usize {
    let parents = parent_pids();
    parents
        .keys()
        .filter(|&&pid| is_webkit_web_process(pid) && is_descendant_of(pid, root, &parents))
        .count()
}

fn wait_for_webkit_process_count_at_most(target: usize, root: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if count_descendant_webkit_processes(root) <= target {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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
    fn mark_startup_logged_once_logs_each_label_exactly_once() {
        let mut logged = HashSet::new();
        assert!(mark_startup_logged_once(&mut logged, "settings"));
        assert!(!mark_startup_logged_once(&mut logged, "settings"));
        assert!(mark_startup_logged_once(&mut logged, "prompt"));
    }

    #[test]
    fn is_descendant_of_walks_up_the_tree() {
        let mut parents = HashMap::new();
        parents.insert(3u32, 2u32);
        parents.insert(2u32, 1u32);
        assert!(is_descendant_of(3, 1, &parents));
        assert!(!is_descendant_of(3, 99, &parents));
    }

    #[test]
    fn is_descendant_of_unknown_pid_is_false() {
        let parents = HashMap::new();
        assert!(!is_descendant_of(3, 1, &parents));
    }
}
