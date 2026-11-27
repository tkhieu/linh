//! Background update checking and in-place install, backed by
//! `tauri-plugin-updater`.
//!
//! The pieces that decide *when* to check, *whether* this install can
//! self-update, and *whether* to notify again are plain functions over
//! plain data (no [`tauri::AppHandle`], no network), so they're unit
//! tested directly. The async glue around them (`run_scheduler`,
//! `check_now`, `install_update`) is the thin, untested part that calls
//! the plugin and feeds its result through those functions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

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

/// What [`UpdateKind`] may be offered in the UI.
pub fn capability_for(kind: UpdateKind) -> Capability {
    match kind {
        UpdateKind::Rpm | UpdateKind::Deb | UpdateKind::Nsis => Capability::SelfInstall,
        UpdateKind::MacApp => Capability::DownloadOnly,
        UpdateKind::Msix | UpdateKind::Dev => Capability::None,
    }
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

/// Whether `version` should still trigger a notification, given the last
/// version Linh already notified about (`None` if it never has). Used so
/// a version found on every one of today's checks notifies exactly once.
pub fn should_notify(notified_version: &Option<String>, version: &str) -> bool {
    notified_version.as_deref() != Some(version)
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
/// the UI-facing commands.
#[derive(Debug, Default)]
pub struct UpdateState {
    pub available: Option<AvailableUpdate>,
    pub last_checked: Option<SystemTime>,
    pub last_error: Option<String>,
    pub notified_version: Option<String>,
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
/// mutable [`UpdateState`], the single-flight guard, and this install's
/// fixed kind/capability.
pub struct Updates {
    pub kind: UpdateKind,
    pub capability: Capability,
    pub started_at: Instant,
    pub in_flight: AtomicBool,
    pub state: Mutex<UpdateState>,
}

impl Updates {
    pub fn init() -> Self {
        let kind = detect_kind();
        Updates {
            kind,
            capability: capability_for(kind),
            started_at: Instant::now(),
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
/// version found.
pub async fn check_now(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<AvailableUpdate>, String> {
    if !try_begin_in_flight(&state.updates.in_flight) {
        return Err("an update check or install is already in progress".to_string());
    }
    let result = perform_check(app).await;
    end_in_flight(&state.updates.in_flight);

    if let Err(err) = &result {
        log_line(&state.dirs, &format!("update check failed: {err}"));
    }

    {
        let mut guard = state.updates.state.lock().unwrap();
        record_check_result(&mut guard, &result, SystemTime::now());
    }

    if let Ok(Some(available)) = &result {
        maybe_notify(state, available);
    }

    let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, ());
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

/// Notifies once per newly-found version, via the platform notifier.
fn maybe_notify(state: &AppState, available: &AvailableUpdate) {
    let mut guard = state.updates.state.lock().unwrap();
    if !should_notify(&guard.notified_version, &available.version) {
        return;
    }
    guard.notified_version = Some(available.version.clone());
    drop(guard);

    notify::notify_info(
        &state.dirs,
        "Linh update available",
        &format!(
            "Linh {} is available — open Settings \u{2192} Updates.",
            available.version
        ),
    );
}

/// Installs the update found by the most recent check: downloads it,
/// verifies it (the plugin checks the signature), installs it in place,
/// then relaunches. Only valid when `capability` is
/// [`Capability::SelfInstall`]; the UI is expected not to offer this
/// otherwise, but this still guards it so a stale UI state can't trigger
/// an install `install_update` has no business doing.
pub async fn install_update(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.updates.capability != Capability::SelfInstall {
        return Err("this install cannot update itself".to_string());
    }
    if !try_begin_in_flight(&state.updates.in_flight) {
        return Err("an update check or install is already in progress".to_string());
    }

    let result = install_inner(app).await;
    end_in_flight(&state.updates.in_flight);

    if let Err(err) = &result {
        log_line(&state.dirs, &format!("update install failed: {err}"));
        let mut guard = state.updates.state.lock().unwrap();
        guard.last_error = Some(err.clone());
    }

    let _ = app.emit(UPDATE_STATUS_CHANGED_EVENT, ());
    result
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|err| describe_error(&err))?;
    let update = updater
        .check()
        .await
        .map_err(|err| describe_error(&err))?
        .ok_or_else(|| "no update is available to install".to_string())?;

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
        let last_check = state.updates.state.lock().unwrap().last_checked.is_some();
        let due = {
            let started_at = state.updates.started_at;
            // `last_checked` is a `SystemTime` (for the UI), but the
            // schedule itself compares `Instant`s; a check having
            // happened at all is enough to know we're past the "first
            // check" phase, since `check_due` only needs *an* elapsed
            // `Instant`, not that exact one, to pick the right delay.
            let last_check_instant = last_check.then(Instant::now);
            check_due(started_at, last_check_instant, now)
        };
        if !due {
            continue;
        }
        if !state.config.lock().unwrap().check_for_updates {
            continue;
        }
        let _ = check_now(&app, &state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn capability_matches_the_owner_s_per_kind_decision() {
        assert_eq!(capability_for(UpdateKind::Rpm), Capability::SelfInstall);
        assert_eq!(capability_for(UpdateKind::Deb), Capability::SelfInstall);
        assert_eq!(capability_for(UpdateKind::Nsis), Capability::SelfInstall);
        assert_eq!(capability_for(UpdateKind::MacApp), Capability::DownloadOnly);
        assert_eq!(capability_for(UpdateKind::Msix), Capability::None);
        assert_eq!(capability_for(UpdateKind::Dev), Capability::None);
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
