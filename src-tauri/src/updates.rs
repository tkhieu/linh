//! Background update checking and in-place install, backed by
//! `tauri-plugin-updater`.
//!
//! The pieces that decide *when* to check, *whether* this install can
//! self-update, and *whether* to notify again are plain functions over
//! plain data (no [`tauri::AppHandle`], no network), so they're unit
//! tested directly. The async glue around them (`run_scheduler`,
//! `check_now`, `install_update`) is the thin, untested part that calls
//! the plugin and feeds its result through those functions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::platform::current::dirs::Dirs;
use crate::platform::current::notify;
use crate::platform::logging::log_line;
#[cfg(target_os = "windows")]
use crate::platform::windows::packaging::is_packaged;
use crate::state::AppState;

/// How this running copy of Linh was installed, decided once at startup
/// (see [`detect_kind`]) and never re-derived: the bundle identity a
/// process has is baked in at build time, not something that changes
/// mid-session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Rpm,
    Deb,
    Nsis,
    /// MSIX/Microsoft Store install. The plugin does not support
    /// updating these; the Store or an `.appinstaller` does.
    Msix,
    /// macOS `.app` bundle, unsigned. Self-install is withheld until
    /// Developer ID signing exists (see the plan's risk list), so this
    /// only ever gets [`Capability::DownloadOnly`].
    MacApp,
    /// A debug build, or a release binary not installed from any of the
    /// bundles above (e.g. run straight from `target/release`).
    Dev,
}

/// What the "Updates" UI may offer for a given [`UpdateKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// "Update now": download, verify (the plugin verifies the
    /// signature), install in place, then relaunch.
    SelfInstall,
    /// "Download": open the release page in the default browser instead
    /// of installing anything.
    DownloadOnly,
    /// Nothing offered beyond the check itself.
    None,
}

/// Pure classification behind [`detect_kind`], taking its inputs as
/// plain values so every branch is unit-testable without `cfg` or real
/// platform calls.
fn classify(is_debug: bool, is_msix: bool, bundle: Option<BundleType>) -> UpdateKind {
    if is_debug {
        return UpdateKind::Dev;
    }
    if is_msix {
        return UpdateKind::Msix;
    }
    match bundle {
        Some(BundleType::Deb) => UpdateKind::Deb,
        Some(BundleType::Rpm) => UpdateKind::Rpm,
        Some(BundleType::Nsis) => UpdateKind::Nsis,
        Some(BundleType::App) => UpdateKind::MacApp,
        // AppImage/Msi/Dmg/None: not a bundle Linh ships (see
        // `tauri.conf.json`'s `bundle.targets`), or no bundle identity at
        // all. Neither is a kind the UI has a plan for, so treat it the
        // same as a dev build: check only, offer nothing.
        _ => UpdateKind::Dev,
    }
}

/// Detects [`UpdateKind`] for the current process.
///
/// Linux and Windows bundle identity comes from
/// [`tauri::utils::platform::bundle_type`], which reads a static string
/// (`__TAURI_BUNDLE_TYPE`) that the Tauri bundler patches into the
/// binary at bundle time — see `tauri-utils`' `platform.rs`. This
/// deliberately does not re-implement that detection; MSIX is the one
/// case it doesn't cover (it has no "bundle type" in that sense), so
/// that's checked separately via `GetCurrentPackageFullName`
/// ([`platform::windows::packaging::is_packaged`]).
pub fn detect_kind() -> UpdateKind {
    #[cfg(target_os = "windows")]
    let is_msix = is_packaged();
    #[cfg(not(target_os = "windows"))]
    let is_msix = false;
    classify(
        cfg!(debug_assertions),
        is_msix,
        tauri::utils::platform::bundle_type(),
    )
}

/// What [`UpdateKind`] may be offered in the UI. `exe_matches_install`
/// (see [`exe_matches_expected_install`]) withholds `SelfInstall` for a
/// kind that would otherwise get it when the running binary isn't
/// actually the one its package manager installed — e.g. a copy of the
/// rpm/deb binary run from elsewhere, which would still `rpm -U`/`dpkg
/// -i` the *real* system install (the bundle-type marker survives being
/// copied) and then relaunch the unchanged copy, not the new install.
/// Ignored for kinds this has nothing to check for (see
/// [`expected_install_path`]).
pub fn capability_for(kind: UpdateKind, exe_matches_install: bool) -> Capability {
    match kind {
        UpdateKind::Rpm | UpdateKind::Deb | UpdateKind::Nsis => {
            if exe_matches_install {
                Capability::SelfInstall
            } else {
                Capability::DownloadOnly
            }
        }
        UpdateKind::MacApp => Capability::DownloadOnly,
        UpdateKind::Msix | UpdateKind::Dev => Capability::None,
    }
}

/// The path a package manager actually installed `kind` to, when Linh
/// can check one. `None` for kinds this has no reliable check for yet.
fn expected_install_path(kind: UpdateKind) -> Option<&'static str> {
    match kind {
        // Confirmed directly from this repo's own rpm bundle:
        // `rpm -qlp target/release/bundle/rpm/linh-*.rpm` lists
        // `/usr/bin/linh`. deb packages built by the same tauri-bundler
        // use the identical `/usr/bin/<binary-name>` convention (not
        // independently re-verified here, since building a `.deb`
        // requires `dpkg-deb`, unavailable on this machine).
        UpdateKind::Rpm | UpdateKind::Deb => Some("/usr/bin/linh"),
        // NSIS (Windows): tauri's NSIS installer records its own
        // install directory in the uninstaller registry key it writes,
        // but which hive/value that is for `installMode: "currentUser"`
        // was not verified against a real install in this change. Left
        // unchecked rather than shipping a guess that could wrongly
        // downgrade every genuine NSIS install's capability — a
        // non-blocking gap, not a decision that this can't ever matter.
        UpdateKind::Nsis | UpdateKind::Msix | UpdateKind::MacApp | UpdateKind::Dev => None,
    }
}

/// Whether the running binary is at the path [`expected_install_path`]
/// says `kind` should be, for a `kind` that has one to check; `true` for
/// a `kind` with nothing to check. See [`capability_for`].
pub fn exe_matches_expected_install(kind: UpdateKind) -> bool {
    exe_matches_expected_install_with(kind, || {
        std::env::current_exe().and_then(|path| path.canonicalize())
    })
}

/// The pure decision behind [`exe_matches_expected_install`], taking the
/// (already-canonicalized, in real use) current exe path as a parameter
/// so it's unit-testable without touching the real filesystem — a
/// literal path like `/usr/bin/linh` passed in a test does not need to
/// actually exist for `canonicalize()` to succeed on it, unlike calling
/// `canonicalize()` inside this function itself would require. Fails
/// closed (`false`, i.e. `DownloadOnly`) if `current_exe` itself errors:
/// the whole point of this check is catching the binary not being where
/// it's supposed to be, so an inability to even determine that is not a
/// reason to assume it's fine.
fn exe_matches_expected_install_with(
    kind: UpdateKind,
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
) -> bool {
    let Some(expected) = expected_install_path(kind) else {
        return true;
    };
    current_exe()
        .map(|path| path == Path::new(expected))
        .unwrap_or(false)
}

/// The first background check happens shortly after startup rather than
/// immediately, so it doesn't compete with first-launch work.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
/// After the first check, every subsequent one is a day apart.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether a background check is due at `now`, given when Linh started
/// and when the last check attempt *finished* (`last_check`; success or
/// failure both count — a network error waits for the next regular tick
/// rather than retrying sooner, which is what keeps this from becoming a
/// retry storm).
pub fn check_due(started_at: Instant, last_check: Option<Instant>, now: Instant) -> bool {
    match last_check {
        None => now.saturating_duration_since(started_at) >= FIRST_CHECK_DELAY,
        Some(last) => now.saturating_duration_since(last) >= CHECK_INTERVAL,
    }
}

/// The scheduler's actual timing state: when Linh started, and the real
/// `Instant` the last check attempt was recorded at — as opposed to
/// re-deriving a fresh `Instant::now()` every tick from an unrelated
/// signal (whether `UpdateState.last_checked`, a `SystemTime`, was
/// `Some`), which is what made the original scheduler fire once at
/// startup and then never again: "now minus last" was always ~0 because
/// "last" was quietly re-stamped to "now" on every single tick. [`tick`]
/// is the only way this changes, and it's exactly what both
/// `run_scheduler` and this module's own tests drive.
#[derive(Debug)]
pub struct Schedule {
    started_at: Instant,
    last_check: Option<Instant>,
}

impl Schedule {
    pub fn new(started_at: Instant) -> Self {
        Schedule {
            started_at,
            last_check: None,
        }
    }

    /// Whether a check is due at `now`, per [`check_due`].
    pub fn is_due(&self, now: Instant) -> bool {
        check_due(self.started_at, self.last_check, now)
    }

    /// Records that a check was attempted at `at` (whether it went on to
    /// succeed or fail — see [`check_due`]'s doc comment).
    pub fn record_checked(&mut self, at: Instant) {
        self.last_check = Some(at);
    }
}

/// Single-flight guard shared by the background scheduler and the UI's
/// "check now": at most one check or install runs at a time. Returns
/// `true` if the caller now holds the slot (and so must call
/// [`end_in_flight`] when done), `false` if another check/install is
/// already running.
pub fn try_begin_in_flight(in_flight: &AtomicBool) -> bool {
    in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Releases the slot taken by [`try_begin_in_flight`].
pub fn end_in_flight(in_flight: &AtomicBool) {
    in_flight.store(false, Ordering::SeqCst);
}

/// Whether `version` should still trigger a notification, given the
/// highest version Linh has already notified about (`None` if it never
/// has). Compared as semver when both sides parse as one, so a version
/// no higher than the one already notified doesn't notify again — in
/// particular, a sequence of A (notify) -> B (notify) -> A (e.g. the
/// endpoint serving a stale manifest again) does not re-notify for A,
/// since A is not greater than the recorded highest, B. Falls back to a
/// plain string inequality when either side doesn't parse as semver.
pub fn should_notify(highest_notified: &Option<String>, version: &str) -> bool {
    let Some(highest) = highest_notified else {
        return true;
    };
    match (Version::parse(highest), Version::parse(version)) {
        (Ok(highest), Ok(version)) => version > highest,
        _ => highest != version,
    }
}

/// The new highest-notified version after notifying for `version`, given
/// the previous `highest_notified` — `version` itself, unless both parse
/// as semver and `version` is not actually the greater of the two (which
/// [`should_notify`] already made true for the cases that matter here,
/// so this mostly just carries `version` forward; the semver re-check
/// guards the one case it doesn't: a non-comparable highest being
/// replaced by a lower version purely because the string differed).
fn merged_highest_notified(highest_notified: &Option<String>, version: &str) -> String {
    match highest_notified {
        Some(highest) => match (Version::parse(highest), Version::parse(version)) {
            (Ok(highest_v), Ok(version_v)) if highest_v > version_v => highest.clone(),
            _ => version.to_string(),
        },
        None => version.to_string(),
    }
}

/// Persisted across restarts (unlike the rest of [`UpdateState`], which
/// is session-only) so a version doesn't re-notify just because Linh was
/// restarted before the user updated.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct NotifiedState {
    highest_notified_version: Option<String>,
}

/// Loads the persisted [`NotifiedState`], tolerating a missing or
/// corrupt file by treating either the same as "never notified" —
/// consistent with every other best-effort load in this codebase (e.g.
/// `platform::linux::app_config::load_or_init`'s handling of a broken
/// config file), and the safe direction for this specific file: the
/// worst a corrupt/missing file costs is one redundant notification, not
/// a stuck or crashing updater.
fn load_notified_state(dirs: &Dirs) -> NotifiedState {
    std::fs::read_to_string(dirs.updates_state_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Best-effort atomic write of `state`, the same "unique temp file,
/// `sync_all`, rename" shape as
/// `platform::linux::app_config::write_atomic`. Errors are swallowed:
/// this is a convenience against re-notifying, not state anything else
/// depends on being durable.
fn save_notified_state(dirs: &Dirs, state: &NotifiedState) {
    let path = dirs.updates_state_path();
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(data) = serde_json::to_string(state) else {
        return;
    };
    let tmp_path = parent.join(format!("updates.json.tmp-{}", std::process::id()));
    let Ok(()) = std::fs::write(&tmp_path, data) else {
        return;
    };
    let _ = std::fs::rename(&tmp_path, &path);
}

/// A release the update endpoint announced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub notes: Option<String>,
    /// RFC 3339, when the endpoint provided one.
    pub date: Option<String>,
}

/// The mutable part of update state, shared between the scheduler and
/// the UI-facing commands. Notify-once dedup is tracked separately, in
/// [`NotifiedState`] — persisted to disk rather than kept only here,
/// since it must survive a restart (see [`load_notified_state`]).
#[derive(Debug, Default)]
pub struct UpdateState {
    pub available: Option<AvailableUpdate>,
    pub last_checked: Option<SystemTime>,
    pub last_error: Option<String>,
}

/// The full snapshot [`crate::commands::get_update_status`] returns to
/// the UI.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current_version: String,
    pub kind: UpdateKind,
    pub capability: Capability,
    /// Milliseconds since the Unix epoch, when a check has run.
    pub last_checked: Option<u64>,
    pub available: Option<AvailableUpdate>,
    pub last_error: Option<String>,
}

/// Everything [`AppState`] needs to carry for the updater: the
/// mutable [`UpdateState`], the single-flight guard, the scheduler's own
/// timing state, and this install's fixed kind/capability.
pub struct Updates {
    pub kind: UpdateKind,
    pub capability: Capability,
    pub schedule: Mutex<Schedule>,
    pub in_flight: AtomicBool,
    pub state: Mutex<UpdateState>,
}

impl Updates {
    pub fn init() -> Self {
        let kind = detect_kind();
        Updates {
            kind,
            capability: capability_for(kind, exe_matches_expected_install(kind)),
            schedule: Mutex::new(Schedule::new(Instant::now())),
            in_flight: AtomicBool::new(false),
            state: Mutex::new(UpdateState::default()),
        }
    }
}

fn system_time_millis(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Builds the snapshot returned by `get_update_status`.
pub fn status_snapshot(updates: &Updates) -> UpdateStatus {
    let state = updates.state.lock().unwrap();
    UpdateStatus {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        kind: updates.kind,
        capability: updates.capability,
        last_checked: state.last_checked.map(system_time_millis),
        available: state.available.clone(),
        last_error: state.last_error.clone(),
    }
}

/// Turns a plugin error into text the UI can show as-is. `#[non_exhaustive]`
/// on `tauri_plugin_updater::Error` means this can never be exhaustive on
/// our side either, so unrecognized variants fall back to their own
/// `Display` message rather than a generic one.
pub fn describe_error(err: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error::*;
    match err {
        PackageInstallFailed => {
            "The installer could not run — the password prompt may have been cancelled or denied. \
             Try \"Update now\" again, or use \"Download\" to install it by hand."
                .to_string()
        }
        AuthenticationFailed => {
            "Authentication was cancelled or failed. Try \"Update now\" again, or use \
             \"Download\" to install it by hand."
                .to_string()
        }
        DebInstallFailed => "The .deb package could not be installed.".to_string(),
        other => other.to_string(),
    }
}

/// Runs one check against the update endpoint and translates the result
/// (or error) into [`AvailableUpdate`] / an error string, without
/// touching [`UpdateState`] — that's the caller's job, so this can be
/// awaited inside the single-flight guard without holding any lock
/// across the `.await`.
async fn perform_check(app: &AppHandle) -> Result<Option<AvailableUpdate>, String> {
    let updater = app.updater().map_err(|err| describe_error(&err))?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(AvailableUpdate {
            version: update.version.clone(),
            notes: update.body.clone(),
            date: update.date.map(|date| date.to_string()),
        })),
        Ok(None) => Ok(None),
        Err(err) => Err(describe_error(&err)),
    }
}

/// Records the outcome of a check into `state`, returning the available
/// update (if any) so the caller can decide whether to notify.
fn record_check_result(
    state: &mut UpdateState,
    result: &Result<Option<AvailableUpdate>, String>,
    checked_at: SystemTime,
) {
    state.last_checked = Some(checked_at);
    match result {
        Ok(available) => {
            state.last_error = None;
            state.available = available.clone();
        }
        Err(err) => {
            state.last_error = Some(err.clone());
        }
    }
}

/// Runs a single-flighted update check: used by both the background
/// scheduler and the UI's "check now". Notifies at most once per new
/// version found. Holds the single-flight guard for the *entire* call,
/// including recording the result and notifying — not just the network
/// request — so a UI-triggered check and a background tick can't
/// interleave their writes to `UpdateState`/[`NotifiedState`] either.
pub async fn check_now(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<AvailableUpdate>, String> {
    if !try_begin_in_flight(&state.updates.in_flight) {
        return Err("an update check or install is already in progress".to_string());
    }

    let result = perform_check(app).await;

    if let Err(err) = &result {
        log_line(&state.dirs, &format!("update check failed: {err}"));
    }

    {
        let mut guard = state.updates.state.lock().unwrap();
        record_check_result(&mut guard, &result, SystemTime::now());
    }

    if let Ok(Some(available)) = &result {
        maybe_notify(app, state, available);
    }

    let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, ());
    end_in_flight(&state.updates.in_flight);
    result
}

/// Emitted whenever `check_now`/`install_update` finish, so the Settings
/// UI knows to re-fetch `get_update_status` rather than polling it.
pub const UPDATE_STATUS_CHANGED_EVENT: &str = "updates://status-changed";

/// Emitted during `install_update`'s download with `(downloaded_bytes,
/// total_bytes)`, `total_bytes` being `None` if the server didn't send a
/// `Content-Length`.
pub const UPDATE_DOWNLOAD_PROGRESS_EVENT: &str = "updates://download-progress";

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: usize,
    total: Option<u64>,
}

/// Notifies once per newly-found version, via a real toast on Windows
/// (see [`notify_via_toast`]) and the platform notifier everywhere else.
/// Dedup state is loaded from and saved to disk (not kept only in
/// memory) so a restart before the user updates doesn't re-notify.
fn maybe_notify(app: &AppHandle, state: &AppState, available: &AvailableUpdate) {
    let mut notified = load_notified_state(&state.dirs);
    if !should_notify(&notified.highest_notified_version, &available.version) {
        return;
    }
    notified.highest_notified_version = Some(merged_highest_notified(
        &notified.highest_notified_version,
        &available.version,
    ));
    save_notified_state(&state.dirs, &notified);

    let summary = "Linh update available";
    let body = format!(
        "Linh {} is available — open Settings \u{2192} Updates.",
        available.version
    );
    if !notify_via_toast(app, summary, &body) {
        notify::notify_info(&state.dirs, summary, &body);
    }
}

/// On Windows, attempts a real toast via `tauri-plugin-notification`
/// (backed by `tauri-winrt-notification`, using the app's own
/// `identifier` from `tauri.conf.json` as its `AppUserModelID`) instead
/// of `notify::notify_info`'s log-only fallback there — the owner wants
/// a visible notice, not just a log line. Returns `true` if the toast
/// was accepted by the OS, so the caller can fall back to
/// `notify::notify_info` when it wasn't.
///
/// This is *not* guaranteed to show anything even when it returns `true`:
/// Windows toast notifications normally require the app's
/// `AppUserModelID` to be registered (typically via a Start Menu
/// shortcut, which Tauri's NSIS installer creates with a matching one),
/// and that registration was not independently verified against a real
/// NSIS install in this change — `.show()` can succeed as an API call
/// while Windows still silently drops an unregistered app's toast.
/// `notify::notify_error`'s own behaviour is deliberately left
/// unchanged: its callers include `AppState::init`, which runs before
/// any `AppHandle` exists to call this with.
#[cfg(target_os = "windows")]
fn notify_via_toast(app: &AppHandle, summary: &str, body: &str) -> bool {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(summary)
        .body(body)
        .show()
        .is_ok()
}

#[cfg(not(target_os = "windows"))]
fn notify_via_toast(_app: &AppHandle, _summary: &str, _body: &str) -> bool {
    false
}

/// Installs the update found when this call re-checks, but only if it is
/// still `expected_version` — the version the UI last showed the user,
/// not whatever happens to be announced *now*. A mismatch (a newer
/// release appeared, or the one the user saw was pulled) is refused
/// rather than silently installed, so the UI can re-check and re-prompt
/// instead of the user ending up on a release they never actually
/// approved. Only valid when `capability` is [`Capability::SelfInstall`];
/// the UI is expected not to offer this otherwise, but this still guards
/// it so a stale UI state can't trigger an install `install_update` has
/// no business doing.
pub async fn install_update(
    app: &AppHandle,
    state: &AppState,
    expected_version: &str,
) -> Result<(), String> {
    if state.updates.capability != Capability::SelfInstall {
        return Err("this install cannot update itself".to_string());
    }
    if !try_begin_in_flight(&state.updates.in_flight) {
        return Err("an update check or install is already in progress".to_string());
    }

    let result = install_inner(app, expected_version).await;

    if let Err(err) = &result {
        log_line(&state.dirs, &format!("update install failed: {err}"));
        let mut guard = state.updates.state.lock().unwrap();
        guard.last_error = Some(err.clone());
    }

    let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, ());
    end_in_flight(&state.updates.in_flight);
    result
}

/// The pure decision behind `install_inner` refusing to install a
/// version other than the one the UI showed the user: `None` when
/// `found_version` (what a fresh check just announced) matches
/// `expected_version`, `Some(message)` otherwise.
fn version_mismatch_error(found_version: &str, expected_version: &str) -> Option<String> {
    if found_version == expected_version {
        None
    } else {
        Some(format!(
            "a different version ({found_version}) is now available; re-check before installing"
        ))
    }
}

async fn install_inner(app: &AppHandle, expected_version: &str) -> Result<(), String> {
    let updater = app.updater().map_err(|err| describe_error(&err))?;
    let update = updater
        .check()
        .await
        .map_err(|err| describe_error(&err))?
        .ok_or_else(|| "no update is available to install".to_string())?;

    if let Some(err) = version_mismatch_error(&update.version, expected_version) {
        return Err(err);
    }

    let progress_app = app.clone();
    update
        .download_and_install(
            move |downloaded, total| {
                let _ = progress_app.emit(
                    UPDATE_DOWNLOAD_PROGRESS_EVENT,
                    DownloadProgress { downloaded, total },
                );
            },
            || {},
        )
        .await
        .map_err(|err| describe_error(&err))?;

    // Windows: the plugin already exits the process after launching the
    // installer, so this is unreached there. macOS/Linux install in
    // place and need an explicit relaunch. `restart` never returns, and
    // its `!` coerces to this function's `Result`.
    app.restart()
}

/// Where "Download" (the UI's action for [`Capability::DownloadOnly`])
/// sends the user: GitHub's permanent "latest release" redirect, the
/// same release the update manifest itself is published against (see
/// the plan's architecture section).
const RELEASE_PAGE_URL: &str = "https://github.com/tkhieu/linh/releases/latest";

/// Opens the release page in the user's configured primary browser —
/// reusing [`crate::routing::open_target`] rather than a generic URL
/// opener, so this goes straight to that browser's binary instead of
/// through Linh's own URL routing (which could otherwise loop back into
/// Linh itself if it's registered as the default browser).
pub fn open_release_page(state: &AppState) -> Result<(), String> {
    let primary = state.config.lock().unwrap().primary.clone();
    crate::routing::open_target(state, &primary, RELEASE_PAGE_URL)
}

/// Runs the background scheduler for the lifetime of the app: waits
/// until a check is due (honouring `config.check_for_updates`, re-read
/// on every tick so toggling it in Settings takes effect without a
/// restart), then runs one single-flighted [`check_now`].
pub async fn run_scheduler(app: AppHandle) {
    let state = app.state::<AppState>();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = Instant::now();
        if !state.updates.schedule.lock().unwrap().is_due(now) {
            continue;
        }
        if !state.config.lock().unwrap().check_for_updates {
            continue;
        }
        // Recorded *before* the check runs (not after), same as the
        // single-flight guard: a check that takes a while must not make
        // the next tick see `is_due` go true again in the meantime.
        state.updates.schedule.lock().unwrap().record_checked(now);
        let _ = check_now(&app, &state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("linh-updates-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn fs_cleanup(root: &std::path::Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
            application_dirs: Vec::new(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn test_dirs(root: &std::path::Path) -> Dirs {
        Dirs {
            config_home: root.join("home/.config"),
            state_home: root.join("home/.local/state"),
        }
    }

    #[test]
    fn debug_builds_are_always_dev_regardless_of_bundle_or_msix() {
        assert_eq!(classify(true, true, Some(BundleType::Rpm)), UpdateKind::Dev);
        assert_eq!(classify(true, false, None), UpdateKind::Dev);
    }

    #[test]
    fn msix_wins_over_bundle_type_when_not_debug() {
        assert_eq!(
            classify(false, true, Some(BundleType::Nsis)),
            UpdateKind::Msix
        );
        assert_eq!(classify(false, true, None), UpdateKind::Msix);
    }

    #[test]
    fn linux_and_windows_bundle_types_map_directly() {
        assert_eq!(
            classify(false, false, Some(BundleType::Deb)),
            UpdateKind::Deb
        );
        assert_eq!(
            classify(false, false, Some(BundleType::Rpm)),
            UpdateKind::Rpm
        );
        assert_eq!(
            classify(false, false, Some(BundleType::Nsis)),
            UpdateKind::Nsis
        );
    }

    #[test]
    fn macos_app_bundle_maps_to_mac_app() {
        assert_eq!(
            classify(false, false, Some(BundleType::App)),
            UpdateKind::MacApp
        );
    }

    #[test]
    fn unrecognized_or_missing_bundle_is_dev() {
        assert_eq!(classify(false, false, None), UpdateKind::Dev);
        assert_eq!(
            classify(false, false, Some(BundleType::AppImage)),
            UpdateKind::Dev
        );
        assert_eq!(
            classify(false, false, Some(BundleType::Msi)),
            UpdateKind::Dev
        );
        assert_eq!(
            classify(false, false, Some(BundleType::Dmg)),
            UpdateKind::Dev
        );
    }

    #[test]
    fn capability_matches_the_owner_s_per_kind_decision_when_the_exe_path_checks_out() {
        assert_eq!(
            capability_for(UpdateKind::Rpm, true),
            Capability::SelfInstall
        );
        assert_eq!(
            capability_for(UpdateKind::Deb, true),
            Capability::SelfInstall
        );
        assert_eq!(
            capability_for(UpdateKind::Nsis, true),
            Capability::SelfInstall
        );
        assert_eq!(
            capability_for(UpdateKind::MacApp, true),
            Capability::DownloadOnly
        );
        assert_eq!(capability_for(UpdateKind::Msix, true), Capability::None);
        assert_eq!(capability_for(UpdateKind::Dev, true), Capability::None);
    }

    #[test]
    fn a_mismatched_exe_path_downgrades_rpm_deb_and_nsis_to_download_only() {
        assert_eq!(
            capability_for(UpdateKind::Rpm, false),
            Capability::DownloadOnly
        );
        assert_eq!(
            capability_for(UpdateKind::Deb, false),
            Capability::DownloadOnly
        );
        assert_eq!(
            capability_for(UpdateKind::Nsis, false),
            Capability::DownloadOnly
        );
    }

    #[test]
    fn a_mismatched_exe_path_does_not_affect_kinds_with_nothing_to_check() {
        // MacApp/Msix/Dev have no `expected_install_path`, so the flag
        // must not change their (already fixed) capability either way.
        assert_eq!(
            capability_for(UpdateKind::MacApp, false),
            Capability::DownloadOnly
        );
        assert_eq!(capability_for(UpdateKind::Msix, false), Capability::None);
        assert_eq!(capability_for(UpdateKind::Dev, false), Capability::None);
    }

    #[test]
    fn exe_at_the_expected_path_matches() {
        assert!(exe_matches_expected_install_with(UpdateKind::Rpm, || Ok(
            PathBuf::from("/usr/bin/linh")
        )));
    }

    #[test]
    fn a_copy_run_from_elsewhere_does_not_match() {
        assert!(!exe_matches_expected_install_with(UpdateKind::Rpm, || Ok(
            PathBuf::from("/home/user/Downloads/linh")
        )));
        assert!(!exe_matches_expected_install_with(UpdateKind::Deb, || Ok(
            PathBuf::from("/tmp/linh")
        )));
    }

    #[test]
    fn a_current_exe_lookup_failure_fails_closed_to_download_only() {
        assert!(!exe_matches_expected_install_with(UpdateKind::Rpm, || {
            Err(std::io::Error::other("no such process"))
        }));
    }

    #[test]
    fn kinds_with_no_expected_path_are_not_checked_at_all() {
        assert!(exe_matches_expected_install_with(UpdateKind::Nsis, || Err(
            std::io::Error::other("never called")
        )));
        assert!(exe_matches_expected_install_with(
            UpdateKind::MacApp,
            || Err(std::io::Error::other("never called"))
        ));
        assert!(exe_matches_expected_install_with(UpdateKind::Dev, || Err(
            std::io::Error::other("never called")
        )));
    }

    #[test]
    fn first_check_is_due_only_after_the_first_check_delay() {
        let started = Instant::now();
        assert!(!check_due(started, None, started));
        assert!(!check_due(
            started,
            None,
            started + FIRST_CHECK_DELAY - Duration::from_secs(1)
        ));
        assert!(check_due(started, None, started + FIRST_CHECK_DELAY));
    }

    #[test]
    fn later_checks_are_due_only_after_the_interval_since_the_last_one() {
        let started = Instant::now();
        let last = started + Duration::from_secs(5);
        assert!(!check_due(started, Some(last), last));
        assert!(!check_due(
            started,
            Some(last),
            last + CHECK_INTERVAL - Duration::from_secs(1)
        ));
        assert!(check_due(started, Some(last), last + CHECK_INTERVAL));
    }

    #[test]
    fn a_network_error_does_not_bring_the_next_check_sooner() {
        // `check_due` only ever sees "when did the last attempt finish",
        // not whether it succeeded — a failed attempt still counts as
        // the last attempt, so this is really the same guarantee as the
        // interval test above, called out explicitly since it's the
        // "no retry storm" requirement.
        let started = Instant::now();
        let failed_at = started + Duration::from_secs(5);
        assert!(!check_due(
            started,
            Some(failed_at),
            failed_at + Duration::from_secs(1)
        ));
    }

    /// Drives the *stateful* `Schedule` itself (not just the pure
    /// `check_due` it's built on) across several simulated days, the way
    /// `run_scheduler`'s loop actually does: call `is_due`, and whenever
    /// it's true, call `record_checked` — exactly what regressed before
    /// this fix, when the "last check" fed back into the decision was a
    /// fresh `Instant::now()` every tick instead of the real moment the
    /// previous check was recorded, so `is_due` could never go true a
    /// second time. 1-hour steps (not `run_scheduler`'s real 1-second
    /// ticks) keep the loop below cheap without losing the behavior:
    /// `is_due`/`record_checked` only ever compare elapsed `Duration`s,
    /// not tick count.
    #[test]
    fn schedule_keeps_firing_about_once_a_day_when_driven_across_multiple_days() {
        let t0 = Instant::now();
        let mut schedule = Schedule::new(t0);
        let mut fired_at = Vec::new();
        let mut now = t0;
        let end = t0 + CHECK_INTERVAL * 3 + Duration::from_secs(3600);

        while now <= end {
            if schedule.is_due(now) {
                schedule.record_checked(now);
                fired_at.push(now.saturating_duration_since(t0));
            }
            now += Duration::from_secs(3600);
        }

        assert_eq!(
            fired_at.len(),
            4,
            "the startup check plus 3 more, one per day, across just over 3 days: {fired_at:?}"
        );
        assert!(
            fired_at[0] >= FIRST_CHECK_DELAY && fired_at[0] <= Duration::from_secs(3600),
            "the first check must fire on the first 1h-stepped tick at or after the 60s \
             startup delay: {:?}",
            fired_at[0]
        );
        for i in 1..fired_at.len() {
            let gap = fired_at[i] - fired_at[i - 1];
            assert!(
                gap >= CHECK_INTERVAL,
                "gap {gap:?} between check {} and {} must be at least a day",
                i - 1,
                i
            );
            assert!(
                gap < CHECK_INTERVAL + Duration::from_secs(3600),
                "gap {gap:?} must not drift far past a day (1h step granularity)"
            );
        }
    }

    #[test]
    fn schedule_does_not_fire_again_before_the_interval_even_when_polled_every_second() {
        // The same property at `run_scheduler`'s real tick granularity,
        // over a shorter span, so the previous (coarser) test's choice
        // of a 1-hour step isn't hiding anything that only shows up at
        // 1-second resolution.
        let t0 = Instant::now();
        let mut schedule = Schedule::new(t0);
        let mut fire_count = 0;
        let mut now = t0;
        let end = t0 + FIRST_CHECK_DELAY + Duration::from_secs(120);

        while now <= end {
            if schedule.is_due(now) {
                schedule.record_checked(now);
                fire_count += 1;
            }
            now += Duration::from_secs(1);
        }

        assert_eq!(
            fire_count, 1,
            "must fire exactly once across this short span, not once per tick"
        );
    }

    #[test]
    fn version_mismatch_refuses_install_of_a_different_version() {
        let err = version_mismatch_error("1.3.0", "1.2.0").expect("must refuse");
        assert!(err.contains("1.3.0"));
    }

    #[test]
    fn version_mismatch_allows_install_of_the_expected_version() {
        assert_eq!(version_mismatch_error("1.2.0", "1.2.0"), None);
    }

    #[test]
    fn single_flight_guard_blocks_a_second_check_until_the_first_ends() {
        let in_flight = AtomicBool::new(false);
        assert!(try_begin_in_flight(&in_flight));
        assert!(
            !try_begin_in_flight(&in_flight),
            "second caller must be refused"
        );
        end_in_flight(&in_flight);
        assert!(
            try_begin_in_flight(&in_flight),
            "slot is free again after end"
        );
    }

    #[test]
    fn notifies_the_first_time_a_version_is_seen() {
        assert!(should_notify(&None, "1.2.3"));
    }

    #[test]
    fn does_not_notify_again_for_the_same_version() {
        assert!(!should_notify(&Some("1.2.3".to_string()), "1.2.3"));
    }

    #[test]
    fn notifies_again_once_a_newer_version_shows_up() {
        assert!(should_notify(&Some("1.2.3".to_string()), "1.3.0"));
    }

    #[test]
    fn does_not_notify_for_a_version_lower_than_the_one_already_notified() {
        // A -> B -> A: having already notified for B (higher), seeing A
        // again (e.g. a stale manifest) must not re-notify for A.
        assert!(!should_notify(&Some("1.3.0".to_string()), "1.2.3"));
    }

    #[test]
    fn semver_comparison_is_numeric_not_lexicographic() {
        // "1.10.0" is lexicographically less than "1.2.0" but numerically
        // greater — this only passes with an actual semver comparison.
        assert!(should_notify(&Some("1.2.0".to_string()), "1.10.0"));
        assert!(!should_notify(&Some("1.10.0".to_string()), "1.2.0"));
    }

    #[test]
    fn non_semver_versions_fall_back_to_string_inequality() {
        assert!(should_notify(&Some("build-42".to_string()), "build-43"));
        assert!(!should_notify(&Some("build-42".to_string()), "build-42"));
    }

    #[test]
    fn merged_highest_tracks_the_higher_of_the_two_semver_versions() {
        assert_eq!(merged_highest_notified(&None, "1.2.3"), "1.2.3");
        assert_eq!(
            merged_highest_notified(&Some("1.2.3".to_string()), "1.3.0"),
            "1.3.0"
        );
        // Never actually reached via `maybe_notify` (which only merges
        // after `should_notify` already said yes), but the merge itself
        // must still not regress the recorded highest if ever called
        // with a lower version directly.
        assert_eq!(
            merged_highest_notified(&Some("1.3.0".to_string()), "1.2.3"),
            "1.3.0"
        );
    }

    #[test]
    fn a_full_restart_cycle_does_not_re_notify_for_the_same_version() {
        // Simulates two separate sessions sharing the same `updates.json`
        // (what the review asked to see "across a simulated restart"):
        // session 1 notifies and persists; session 2, starting from
        // nothing in memory, loads that file and must refuse to notify
        // again for the same version.
        let root = temp_root("restart-cycle");
        let dirs = test_dirs(&root);

        let mut session1 = load_notified_state(&dirs);
        assert_eq!(session1, NotifiedState::default(), "nothing persisted yet");
        assert!(should_notify(&session1.highest_notified_version, "1.2.3"));
        session1.highest_notified_version = Some(merged_highest_notified(
            &session1.highest_notified_version,
            "1.2.3",
        ));
        save_notified_state(&dirs, &session1);

        let session2 = load_notified_state(&dirs);
        assert_eq!(session2.highest_notified_version, Some("1.2.3".to_string()));
        assert!(
            !should_notify(&session2.highest_notified_version, "1.2.3"),
            "a fresh session loading persisted state must not re-notify"
        );

        fs_cleanup(&root);
    }

    #[test]
    fn a_corrupt_updates_json_is_tolerated_as_never_notified() {
        let root = temp_root("corrupt-state");
        let dirs = test_dirs(&root);
        let path = dirs.updates_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is not valid json {{{").unwrap();

        let state = load_notified_state(&dirs);
        assert_eq!(state, NotifiedState::default());

        fs_cleanup(&root);
    }

    #[test]
    fn a_missing_updates_json_is_tolerated_as_never_notified() {
        let root = temp_root("missing-state");
        let dirs = test_dirs(&root);
        assert_eq!(load_notified_state(&dirs), NotifiedState::default());
        fs_cleanup(&root);
    }

    #[test]
    fn save_then_load_round_trips() {
        let root = temp_root("round-trip");
        let dirs = test_dirs(&root);
        let state = NotifiedState {
            highest_notified_version: Some("2.0.0".to_string()),
        };
        save_notified_state(&dirs, &state);
        assert_eq!(load_notified_state(&dirs), state);
        fs_cleanup(&root);
    }

    #[test]
    fn record_check_result_stores_the_available_update_and_clears_old_errors() {
        let mut state = UpdateState {
            last_error: Some("stale error".to_string()),
            ..UpdateState::default()
        };
        let available = AvailableUpdate {
            version: "1.2.3".to_string(),
            notes: Some("notes".to_string()),
            date: None,
        };
        let now = SystemTime::now();
        record_check_result(&mut state, &Ok(Some(available.clone())), now);

        assert_eq!(state.available, Some(available));
        assert_eq!(state.last_error, None);
        assert_eq!(state.last_checked, Some(now));
    }

    #[test]
    fn record_check_result_keeps_the_previous_available_update_on_error() {
        let available = AvailableUpdate {
            version: "1.2.3".to_string(),
            notes: None,
            date: None,
        };
        let mut state = UpdateState {
            available: Some(available.clone()),
            ..UpdateState::default()
        };
        record_check_result(
            &mut state,
            &Err("network down".to_string()),
            SystemTime::now(),
        );

        assert_eq!(state.available, Some(available));
        assert_eq!(state.last_error, Some("network down".to_string()));
    }

    #[test]
    fn describes_a_cancelled_install_without_the_raw_error_text() {
        let message = describe_error(&tauri_plugin_updater::Error::PackageInstallFailed);
        assert!(message.contains("password prompt"));
    }

    #[test]
    fn describes_a_cancelled_authentication_prompt() {
        let message = describe_error(&tauri_plugin_updater::Error::AuthenticationFailed);
        assert!(message.contains("Authentication"));
    }

    #[test]
    fn unrecognized_errors_fall_back_to_their_own_message() {
        let message = describe_error(&tauri_plugin_updater::Error::ReleaseNotFound);
        assert_eq!(
            message,
            tauri_plugin_updater::Error::ReleaseNotFound.to_string()
        );
    }
}
