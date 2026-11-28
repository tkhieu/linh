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
#[cfg(feature = "self-update")]
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

/// The build-time distribution channel, baked in by `build.rs` from the
/// `LINH_DIST` environment variable (default `github` when unset) via
/// `cargo:rustc-env=LINH_DIST=...`; see `docs/release.md` for the exact
/// build command per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dist {
    /// The GitHub release channel: Linh's own in-app updater is this
    /// install's only update path.
    Github,
    Msstore,
    Mas,
    Flathub,
    Copr,
    Ppa,
}

impl Dist {
    /// What the "Updates" UI should attribute update delivery to for this
    /// channel; `None` only for [`Dist::Github`], the one channel where
    /// Linh's own updater still runs.
    pub fn managed_by(self) -> Option<ManagedBy> {
        match self {
            Dist::Github => None,
            Dist::Msstore => Some(ManagedBy::MicrosoftStore),
            Dist::Mas => Some(ManagedBy::MacAppStore),
            Dist::Flathub => Some(ManagedBy::Flathub),
            Dist::Copr => Some(ManagedBy::Copr),
            Dist::Ppa => Some(ManagedBy::Ppa),
        }
    }
}

/// Parses a `LINH_DIST` value. Takes the raw string as a parameter, rather
/// than reading the environment itself, so the mapping is unit-tested
/// directly without depending on how this process was actually built —
/// [`current_dist`] is the only real caller that reads the compiled-in
/// value.
pub fn parse_dist(raw: &str) -> Option<Dist> {
    match raw {
        "github" => Some(Dist::Github),
        "msstore" => Some(Dist::Msstore),
        "mas" => Some(Dist::Mas),
        "flathub" => Some(Dist::Flathub),
        "copr" => Some(Dist::Copr),
        "ppa" => Some(Dist::Ppa),
        _ => None,
    }
}

/// This build's distribution channel. `build.rs` already validated
/// `LINH_DIST` before compiling this crate at all (see its own `panic!`),
/// so the `expect` below only ever fires if that invariant is broken.
pub fn current_dist() -> Dist {
    parse_dist(env!("LINH_DIST")).expect("build.rs validates LINH_DIST before compilation")
}

/// The exact text `build.rs`'s `write_dist_stamp` writes to the
/// `linh.dist-marker.txt` file it places next to the compiled binary,
/// naming this build's `LINH_DIST`/`self-update` state.
/// `scripts/package-msix.ps1` reads that stamp file before packaging a
/// build into the Store MSIX — not the exe itself: running it crashed in
/// real CI (`linh` is a GUI-subsystem binary, `windows_subsystem =
/// "windows"`, with no reliable stdout when spawned non-interactively),
/// and embedding a marker directly in the exe's own data section (tried
/// next, via a `#[used] static`) also failed in real CI — it survived
/// the Linux dev build this was first checked against, but the MSVC
/// linker's default `/OPT:REF` silently stripped the unreferenced static
/// from the actual Windows build.
///
/// `build.rs` can't call this directly — build scripts compile
/// separately from (and before) the crate they build — so it keeps its
/// own copy of this exact format by hand; this function exists only so
/// that copy has a tested, documented reference to be kept in sync
/// with (hence `#[cfg(test)]`: nothing at runtime ever calls it).
#[cfg(test)]
fn dist_stamp_contents(dist: &str, self_update_enabled: bool) -> String {
    format!("LINH_BUILD_MARKER:dist={dist};self_update={self_update_enabled}\n")
}

/// Where the "Updates" UI says updates come from, for a channel whose
/// [`Dist::managed_by`] is not `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedBy {
    MicrosoftStore,
    MacAppStore,
    Flathub,
    Copr,
    Ppa,
}

/// Forces [`Capability::None`] for any `dist` that isn't [`Dist::Github`],
/// regardless of what [`capability_for`] would otherwise grant `base` —
/// a store or distro-managed channel never offers "Update now"/"Download"
/// from inside Linh, no matter the detected [`UpdateKind`].
pub fn capability_override_for_dist(dist: Dist, base: Capability) -> Capability {
    match dist.managed_by() {
        Some(_) => Capability::None,
        None => base,
    }
}

/// Whether the background scheduler may run at all for this install:
/// `false` for every managed channel (see [`Dist::managed_by`]), so Linh
/// never contacts the update endpoint from a build the store or a distro
/// already updates for.
pub fn scheduler_enabled(managed_by: Option<ManagedBy>) -> bool {
    managed_by.is_none()
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
/// [`expected_exe_path`]).
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

/// The exe path a package manager/installer actually put `kind` at, when
/// Linh can determine one. `None` for a `kind` with nothing to check.
/// `nsis_install_location` is injected (see
/// [`exe_matches_expected_install_with`]) rather than read here directly.
fn expected_exe_path(
    kind: UpdateKind,
    nsis_install_location: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    match kind {
        // Confirmed directly from this repo's own rpm bundle:
        // `rpm -qlp target/release/bundle/rpm/linh-*.rpm` lists
        // `/usr/bin/linh`. deb packages built by the same tauri-bundler
        // use the identical `/usr/bin/<binary-name>` convention (not
        // independently re-verified here, since building a `.deb`
        // requires `dpkg-deb`, unavailable on this machine).
        UpdateKind::Rpm | UpdateKind::Deb => Some(PathBuf::from("/usr/bin/linh")),
        // NSIS (Windows): tauri-bundler's NSIS template writes its
        // install directory to `InstallLocation` under the uninstall
        // registry key it creates for the app, as the literal path
        // wrapped in quote characters (tauri-bundler 2.10.4
        // `src/bundle/windows/nsis/installer.nsi:67,750`; see
        // `nsis_install_location`/`strip_surrounding_quotes`). The
        // binary name matches `tauri.conf.json`'s bin target, `linh`.
        //
        // Joined with a literal `\` (not `PathBuf::join`, which would
        // use `/` when this is compiled for a non-Windows test host)
        // so the result is a genuine Windows path regardless of host:
        // `paths_equal_windows_style` below is pure string logic and
        // runs (and is tested) on every host, not only a real Windows
        // build.
        UpdateKind::Nsis => nsis_install_location().map(|dir| {
            let dir = dir.to_string_lossy();
            let dir = dir.trim_end_matches(['\\', '/']);
            PathBuf::from(format!(r"{dir}\linh.exe"))
        }),
        UpdateKind::Msix | UpdateKind::MacApp | UpdateKind::Dev => None,
    }
}

/// Reads the install directory tauri-bundler's NSIS installer recorded
/// for itself: the `InstallLocation` value under the uninstall key it
/// writes at `Software\Microsoft\Windows\CurrentVersion\Uninstall\linh`
/// (`linh` being `tauri.conf.json`'s `productName`, which is the key
/// name tauri-bundler's NSIS template uses). Tried under
/// `HKEY_CURRENT_USER` first — matching `installMode: "currentUser"` in
/// `tauri.conf.json` — then `HKEY_LOCAL_MACHINE`, in case that ever
/// changes to a per-machine install. `None` if neither hive has it,
/// which [`exe_matches_expected_install_with`] then treats the same as
/// "can't verify" (`DownloadOnly`), not "assume it's fine".
#[cfg(target_os = "windows")]
fn nsis_install_location() -> Option<PathBuf> {
    use crate::platform::windows::registry;
    const UNINSTALL_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\linh";
    let raw = registry::read_string(
        &registry::current_user(),
        UNINSTALL_SUBKEY,
        "InstallLocation",
    )
    .or_else(|| {
        registry::read_string(
            &registry::local_machine(),
            UNINSTALL_SUBKEY,
            "InstallLocation",
        )
    })?;
    Some(PathBuf::from(strip_surrounding_quotes(&raw)))
}

/// Never actually called off Windows: [`UpdateKind::Nsis`] is never
/// returned by [`detect_kind`] there, and [`expected_exe_path`] only
/// invokes this closure for that one kind.
#[cfg(not(target_os = "windows"))]
fn nsis_install_location() -> Option<PathBuf> {
    None
}

/// Strips surrounding whitespace and, if present, one pair of
/// surrounding double quotes. tauri-bundler 2.10.4's NSIS template
/// writes `InstallLocation` as `WriteRegStr ... "InstallLocation"
/// "$\"$INSTDIR$\""` (`installer.nsi:750`) — i.e. the stored value is
/// the path *with literal quote characters around it*, not a plain
/// path string.
///
/// Only ever called from [`nsis_install_location`]'s Windows build —
/// `#[cfg(any(target_os = "windows", test))]` so it (and its own tests)
/// still compile on every other host instead of tripping `dead_code`
/// there.
#[cfg(any(target_os = "windows", test))]
fn strip_surrounding_quotes(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
}

/// Whether the running binary is at the path [`expected_exe_path`] says
/// `kind` should be, for a `kind` that has one to check; `true` for a
/// `kind` with nothing to check. See [`capability_for`].
pub fn exe_matches_expected_install(kind: UpdateKind) -> bool {
    exe_matches_expected_install_with(
        kind,
        || std::env::current_exe().and_then(|path| path.canonicalize()),
        nsis_install_location,
    )
}

/// The pure decision behind [`exe_matches_expected_install`], taking the
/// (already-canonicalized, in real use) current exe path and the NSIS
/// install-location lookup as parameters so it's unit-testable without
/// touching the real filesystem or the real registry — a literal path
/// like `/usr/bin/linh` passed in a test does not need to actually exist
/// for `canonicalize()` to succeed on it, unlike calling `canonicalize()`
/// inside this function itself would require. Fails closed (`false`,
/// i.e. `DownloadOnly`) if `current_exe` errors, or if there's an
/// `expected_exe_path` to check but it comes back `None` (the NSIS
/// registry lookup failed): the whole point of this check is catching
/// the binary not being where it's supposed to be, so being unable to
/// even determine that is not a reason to assume it's fine.
fn exe_matches_expected_install_with(
    kind: UpdateKind,
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
    nsis_install_location: impl FnOnce() -> Option<PathBuf>,
) -> bool {
    // `expected_exe_path` returning `None` means two different things
    // depending on `kind`: "nothing to check" (Msix/MacApp/Dev — always
    // fine), or "there was something to check but the lookup failed"
    // (Nsis with no registry entry — fails closed). Distinguish them by
    // asking again whether `kind` is one that has a check at all.
    if !matches!(kind, UpdateKind::Rpm | UpdateKind::Deb | UpdateKind::Nsis) {
        return true;
    }
    let Some(expected) = expected_exe_path(kind, nsis_install_location) else {
        return false;
    };
    let Ok(actual) = current_exe() else {
        return false;
    };
    if kind == UpdateKind::Nsis {
        paths_equal_windows_style(&actual, &expected)
    } else {
        actual == expected
    }
}

/// Compares two paths the way Windows itself would treat them as "the
/// same path", which a plain `PathBuf` equality does not:
/// case-insensitively (NTFS and the shell are case-insensitive; a
/// registry-recorded `InstallLocation` need not match `current_exe()`'s
/// own casing), and after stripping either side's `\\?\` ("verbatim")
/// prefix — which `Path::canonicalize()` adds on Windows but a plain
/// `InstallLocation` registry value never has, so without stripping it
/// a genuine install's own exe would never match itself.
fn paths_equal_windows_style(a: &Path, b: &Path) -> bool {
    strip_verbatim_prefix(a).to_string_lossy().to_lowercase()
        == strip_verbatim_prefix(b).to_string_lossy().to_lowercase()
}

/// Strips a leading `\\?\` (or `\\?\UNC\`) verbatim-path prefix, if
/// present; returns `path` unchanged otherwise. `\\?\UNC\server\share`
/// becomes `\\server\share`; `\\?\C:\x` becomes `C:\x`.
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
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
    let Ok(mut file) = std::fs::File::create(&tmp_path) else {
        return;
    };
    if std::io::Write::write_all(&mut file, data.as_bytes()).is_err() {
        return;
    }
    if file.sync_all().is_err() {
        return;
    }
    drop(file);
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
    /// `Some` for a store/distro-managed [`Dist`] (see
    /// [`Dist::managed_by`]); the UI shows this instead of offering any
    /// checking or installing of its own.
    pub managed_by: Option<ManagedBy>,
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
    /// This build's `LINH_DIST` managed-channel attribution, if any; see
    /// [`Dist::managed_by`].
    pub managed_by: Option<ManagedBy>,
    pub schedule: Mutex<Schedule>,
    pub in_flight: AtomicBool,
    pub state: Mutex<UpdateState>,
}

impl Updates {
    pub fn init() -> Self {
        let kind = detect_kind();
        let dist = current_dist();
        let managed_by = dist.managed_by();
        let base_capability = capability_for(kind, exe_matches_expected_install(kind));
        Updates {
            kind,
            capability: capability_override_for_dist(dist, base_capability),
            managed_by,
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
        managed_by: updates.managed_by,
        last_checked: state.last_checked.map(system_time_millis),
        available: state.available.clone(),
        last_error: state.last_error.clone(),
    }
}

/// Turns a plugin error into text the UI can show as-is. `#[non_exhaustive]`
/// on `tauri_plugin_updater::Error` means this can never be exhaustive on
/// our side either, so unrecognized variants fall back to their own
/// `Display` message rather than a generic one.
#[cfg(feature = "self-update")]
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
#[cfg(feature = "self-update")]
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

/// The `self-update`-off counterpart of the real [`perform_check`] above:
/// `tauri_plugin_updater` isn't even a dependency in this build (store
/// policy forbids it — see the feature's doc comment in Cargo.toml), so
/// there is no updater to call; this always refuses instead of silently
/// doing nothing, so a caller can't mistake it for "checked, nothing
/// found".
#[cfg(not(feature = "self-update"))]
async fn perform_check(_app: &AppHandle) -> Result<Option<AvailableUpdate>, String> {
    Err("this build does not check for updates; updates are managed outside Linh".to_string())
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
    if state.updates.managed_by.is_some() {
        return Err("updates for this install are managed outside Linh".to_string());
    }
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
#[cfg(feature = "self-update")]
pub const UPDATE_DOWNLOAD_PROGRESS_EVENT: &str = "updates://download-progress";

#[cfg(feature = "self-update")]
#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: usize,
    total: Option<u64>,
}

/// Notifies once per newly-found version: always via the platform
/// notifier (`notify::notify_info` — on Windows this logs the notice, see
/// its own module), and *additionally* via a real toast attempt on
/// Windows (see [`notify_via_toast`]) — not one or the other, since the
/// toast's own success tells us nothing reliable (see its doc comment).
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
    notify::notify_info(&state.dirs, summary, &body);
    notify_via_toast(app, summary, &body);
}

/// On Windows, attempts a real toast via `tauri-plugin-notification`
/// (backed by `tauri-winrt-notification`, using the app's own
/// `identifier` from `tauri.conf.json` as its `AppUserModelID`) — the
/// owner wants a visible notice, not just a log line.
///
/// `.show()` returning `Ok` only means the OS *accepted the request to
/// schedule* the toast, not that it was actually displayed — so this is
/// not treated as a reliable success/failure signal anywhere, and
/// [`maybe_notify`] always logs the notice via `notify::notify_info`
/// regardless of what this returns (or whether it's even called, on
/// non-Windows). Windows toast notifications also normally require the
/// app's `AppUserModelID` to be registered (typically via a Start Menu
/// shortcut, which Tauri's NSIS installer creates with a matching one),
/// and that registration was not independently verified against a real
/// NSIS install in this change. `notify::notify_error`'s own behaviour
/// is deliberately left unchanged: its callers include `AppState::init`,
/// which runs before any `AppHandle` exists to call this with.
#[cfg(target_os = "windows")]
fn notify_via_toast(app: &AppHandle, summary: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title(summary)
        .body(body)
        .show();
}

#[cfg(not(target_os = "windows"))]
fn notify_via_toast(_app: &AppHandle, _summary: &str, _body: &str) {}

/// Installs the update, but only if `expected_version` is both (a) the
/// version [`UpdateState::available`] actually has stored — i.e. the one
/// the UI was actually shown via `get_update_status`/
/// `check_for_updates_now`, not merely whatever string a caller passes —
/// and (b) still what a fresh re-check announces right before
/// installing. Either mismatch is refused rather than silently
/// installing something else, so the UI can re-check and re-prompt
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
    {
        let guard = state.updates.state.lock().unwrap();
        confirm_expected_matches_stored(&guard.available, expected_version)?;
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

/// The pure decision behind [`install_update`] refusing to install
/// anything the UI wasn't actually shown: `Ok(())` only when `stored`
/// holds an update and its version equals `expected_version`; `Err`
/// (with a message fit to show the user) when nothing is stored, or when
/// it doesn't match.
fn confirm_expected_matches_stored(
    stored: &Option<AvailableUpdate>,
    expected_version: &str,
) -> Result<(), String> {
    match stored {
        None => Err("no update is available to install".to_string()),
        Some(available) if available.version != expected_version => Err(format!(
            "the update shown ({}) is not the one requested ({expected_version}); re-check before installing",
            available.version
        )),
        Some(_) => Ok(()),
    }
}

/// The pure decision behind `install_inner` refusing to install a
/// version other than the one the UI showed the user: `None` when
/// `found_version` (what a fresh re-check, right before installing,
/// just announced) matches `expected_version`, `Some(message)`
/// otherwise. This re-check happens *after*
/// [`confirm_expected_matches_stored`] already approved
/// `expected_version` against the last stored check — it exists for the
/// gap between that approval and this moment, not to replace it.
#[cfg(feature = "self-update")]
fn version_mismatch_error(found_version: &str, expected_version: &str) -> Option<String> {
    if found_version == expected_version {
        None
    } else {
        Some(format!(
            "a different version ({found_version}) is now available; re-check before installing"
        ))
    }
}

#[cfg(feature = "self-update")]
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

/// The `self-update`-off counterpart of the real [`install_inner`] above —
/// see [`perform_check`]'s off counterpart for why this refuses outright
/// rather than doing nothing. Unreachable in practice: [`install_update`]
/// only ever calls this once `capability == SelfInstall`, which
/// [`capability_override_for_dist`] never grants for a channel that would
/// also force this feature off (see `build.rs`'s own check), but this
/// still exists so the crate compiles with the feature off at all.
#[cfg(not(feature = "self-update"))]
async fn install_inner(_app: &AppHandle, _expected_version: &str) -> Result<(), String> {
    Err("this build cannot install updates itself; updates are managed outside Linh".to_string())
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

/// The pure decision behind one `run_scheduler` tick: whether `schedule`
/// says a check is due at `now` *and* `check_enabled()` (reading
/// `config.check_for_updates` for real) says checks aren't turned off.
/// When both are true, this records the check as attempted at `now`
/// *before* returning — same as the single-flight guard: a check that
/// then takes a while (or fails) must not make the very next tick see
/// `is_due` go true again, which is what actually keeps this from
/// becoming a retry storm, not anything about the check's own outcome.
/// `check_enabled` is only consulted once a check is actually due — a
/// disabled toggle is never asked about, and so never gets the chance to
/// consume a due check it didn't actually run; it's still due the
/// moment the toggle comes back on.
///
/// Takes `schedule`/`now`/`check_enabled` as parameters (rather than
/// reading `state.updates.schedule`/`Instant::now()`/`state.config`
/// directly) so this is the one piece of `run_scheduler` that's
/// unit-tested by driving it with simulated time across days, instead
/// of a real `tokio::time::sleep` loop and a real clock.
fn scheduler_tick(
    schedule: &mut Schedule,
    now: Instant,
    check_enabled: impl FnOnce() -> bool,
) -> bool {
    if !schedule.is_due(now) {
        return false;
    }
    if !check_enabled() {
        return false;
    }
    schedule.record_checked(now);
    true
}

/// Runs the background scheduler for the lifetime of the app: waits
/// until a check is due (honouring `config.check_for_updates`, re-read
/// on every tick so toggling it in Settings takes effect without a
/// restart), then runs one single-flighted [`check_now`]. The decision
/// itself is [`scheduler_tick`]; this is just the real clock/config/
/// network it's driven with.
pub async fn run_scheduler(app: AppHandle) {
    let state = app.state::<AppState>();
    // Never loops, never sleeps, never touches the network for a
    // store/distro-managed channel — see [`scheduler_enabled`].
    if !scheduler_enabled(state.updates.managed_by) {
        return;
    }
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = Instant::now();
        let should_check = {
            let mut schedule = state.updates.schedule.lock().unwrap();
            scheduler_tick(&mut schedule, now, || {
                state.config.lock().unwrap().check_for_updates
            })
        };
        if should_check {
            let _ = check_now(&app, &state).await;
        }
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

    /// No test below for Rpm/Deb/MacApp/Dev actually needs a real NSIS
    /// install-location lookup, so they all pass this: it panics if
    /// called, proving `expected_exe_path` never invokes it for those
    /// kinds.
    fn never_called_nsis_lookup() -> Option<PathBuf> {
        panic!("nsis_install_location must not be called for a non-NSIS kind")
    }

    #[test]
    fn exe_at_the_expected_path_matches() {
        assert!(exe_matches_expected_install_with(
            UpdateKind::Rpm,
            || Ok(PathBuf::from("/usr/bin/linh")),
            never_called_nsis_lookup
        ));
    }

    #[test]
    fn a_copy_run_from_elsewhere_does_not_match() {
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Rpm,
            || Ok(PathBuf::from("/home/user/Downloads/linh")),
            never_called_nsis_lookup
        ));
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Deb,
            || Ok(PathBuf::from("/tmp/linh")),
            never_called_nsis_lookup
        ));
    }

    #[test]
    fn a_current_exe_lookup_failure_fails_closed_to_download_only() {
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Rpm,
            || Err(std::io::Error::other("no such process")),
            never_called_nsis_lookup
        ));
    }

    #[test]
    fn kinds_with_no_expected_path_are_not_checked_at_all() {
        let never_called_exe = || -> std::io::Result<PathBuf> {
            panic!("current_exe must not be called for a kind with nothing to check")
        };
        assert!(exe_matches_expected_install_with(
            UpdateKind::MacApp,
            never_called_exe,
            never_called_nsis_lookup
        ));
        assert!(exe_matches_expected_install_with(
            UpdateKind::Dev,
            never_called_exe,
            never_called_nsis_lookup
        ));
    }

    // These all route through `paths_equal_windows_style`, which is pure
    // string logic keyed on `kind == Nsis` rather than the *host's*
    // target OS — so, unlike an actual registry read, they run (and are
    // meaningful) on every test host, not only a real Windows build.

    #[test]
    fn nsis_exe_inside_the_registered_install_location_matches() {
        assert!(exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"C:\Users\user\AppData\Local\linh\linh.exe")),
            || Some(PathBuf::from(r"C:\Users\user\AppData\Local\linh"))
        ));
    }

    #[test]
    fn nsis_exe_outside_the_registered_install_location_does_not_match() {
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"C:\Users\user\Downloads\linh.exe")),
            || Some(PathBuf::from(r"C:\Users\user\AppData\Local\linh"))
        ));
    }

    #[test]
    fn nsis_fails_closed_to_download_only_when_the_registry_lookup_comes_up_empty() {
        // "can't verify" must behave the same as "doesn't match" — never
        // "assume it's fine and grant self-install anyway".
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"C:\Users\user\AppData\Local\linh\linh.exe")),
            || None
        ));
    }

    #[test]
    fn nsis_install_location_with_the_literal_quotes_tauri_bundler_writes_still_matches() {
        // `nsis_install_location` strips these before `expected_exe_path`
        // ever sees the value (see `strip_surrounding_quotes`'s own
        // tests); this exercises the *comparison* with a value as if
        // that stripping had already produced a plain path, same as it
        // would for a genuine install.
        assert!(exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"C:\Users\user\AppData\Local\linh\linh.exe")),
            || Some(PathBuf::from(strip_surrounding_quotes(
                r#""C:\Users\user\AppData\Local\linh""#
            )))
        ));
    }

    #[test]
    fn nsis_matches_despite_a_verbatim_prefixed_current_exe() {
        // `Path::canonicalize()` (what `exe_matches_expected_install`'s
        // real `current_exe` closure uses) adds a `\\?\` prefix on
        // Windows; a plain registry `InstallLocation` value never has
        // one. Without stripping it, a genuine install's own exe would
        // never match itself.
        assert!(exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(
                r"\\?\C:\Users\user\AppData\Local\linh\linh.exe"
            )),
            || Some(PathBuf::from(r"C:\Users\user\AppData\Local\linh"))
        ));
    }

    #[test]
    fn nsis_matches_despite_differing_case() {
        // NTFS and the shell are case-insensitive; a registry-recorded
        // `InstallLocation` need not match `current_exe()`'s own casing.
        assert!(exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"C:\USERS\user\APPDATA\Local\LINH\Linh.EXE")),
            || Some(PathBuf::from(r"C:\Users\user\AppData\Local\linh"))
        ));
    }

    #[test]
    fn nsis_rejects_a_genuinely_different_directory_even_with_a_verbatim_prefix_and_case_difference(
    ) {
        // The tolerance added for the verbatim prefix and case must not
        // widen into tolerating an actually different install — this
        // combines both quirks with a real mismatch to prove that.
        assert!(!exe_matches_expected_install_with(
            UpdateKind::Nsis,
            || Ok(PathBuf::from(r"\\?\C:\Users\user\DOWNLOADS\linh.exe")),
            || Some(PathBuf::from(r"C:\Users\user\AppData\Local\linh"))
        ));
    }

    #[test]
    fn strip_surrounding_quotes_removes_one_pair_and_trims_whitespace() {
        assert_eq!(
            strip_surrounding_quotes(r#"  "C:\Program Files\linh"  "#),
            r"C:\Program Files\linh"
        );
    }

    #[test]
    fn strip_surrounding_quotes_leaves_an_unquoted_value_unchanged() {
        assert_eq!(
            strip_surrounding_quotes(r"C:\Program Files\linh"),
            r"C:\Program Files\linh"
        );
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

    /// Drives `run_scheduler`'s actual loop-body decision —
    /// `scheduler_tick`, not just the `Schedule`/`check_due` it's built
    /// on — across several simulated days, with checks always enabled.
    /// Same 1-hour-step shape as `schedule_keeps_firing_about_once_a_day…`
    /// above, now through the function `run_scheduler` itself calls.
    #[test]
    fn scheduler_tick_fires_after_the_startup_delay_then_about_once_a_day() {
        let t0 = Instant::now();
        let mut schedule = Schedule::new(t0);
        let mut fired_at = Vec::new();
        let mut now = t0;
        let end = t0 + CHECK_INTERVAL * 3 + Duration::from_secs(3600);

        while now <= end {
            if scheduler_tick(&mut schedule, now, || true) {
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
        }
    }

    #[test]
    fn scheduler_tick_never_fires_while_the_config_toggle_is_off_but_still_fires_once_re_enabled() {
        let t0 = Instant::now();
        let mut schedule = Schedule::new(t0);
        let due_at = t0 + FIRST_CHECK_DELAY;

        // A due check is asked about on every tick regardless, so this
        // also proves the toggle is re-read every time, not cached from
        // `Schedule::new` or the first call.
        for _ in 0..5 {
            assert!(!scheduler_tick(&mut schedule, due_at, || false));
        }
        assert!(
            scheduler_tick(&mut schedule, due_at, || true),
            "a due check that was only ever refused by the toggle must still be due once \
             it's back on, at the very same instant"
        );
    }

    #[test]
    fn scheduler_tick_recording_the_attempt_up_front_prevents_a_retry_storm() {
        // `scheduler_tick` has no idea whether the `check_now` its caller
        // runs afterward succeeds or fails — by the time that's known,
        // `record_checked` has already run. This is what actually
        // guarantees "a network error does not cause a retry storm": the
        // next tick is refused by `is_due` the same way regardless of
        // what the un-simulated network call would have done.
        let t0 = Instant::now();
        let mut schedule = Schedule::new(t0);
        let first_fire_at = t0 + FIRST_CHECK_DELAY;
        assert!(scheduler_tick(&mut schedule, first_fire_at, || true));

        for seconds in 1..CHECK_INTERVAL.as_secs() {
            let now = first_fire_at + Duration::from_secs(seconds);
            assert!(
                !scheduler_tick(&mut schedule, now, || true),
                "must not fire again anywhere in the day after the first fire, \
                 whether or not the resulting check would have failed"
            );
        }
        assert!(scheduler_tick(
            &mut schedule,
            first_fire_at + CHECK_INTERVAL,
            || true
        ));
    }

    #[test]
    #[cfg(feature = "self-update")]
    fn version_mismatch_refuses_install_of_a_different_version() {
        let err = version_mismatch_error("1.3.0", "1.2.0").expect("must refuse");
        assert!(err.contains("1.3.0"));
    }

    #[test]
    #[cfg(feature = "self-update")]
    fn version_mismatch_allows_install_of_the_expected_version() {
        assert_eq!(version_mismatch_error("1.2.0", "1.2.0"), None);
    }

    fn available(version: &str) -> AvailableUpdate {
        AvailableUpdate {
            version: version.to_string(),
            notes: None,
            date: None,
        }
    }

    #[test]
    fn confirm_expected_refuses_when_nothing_is_stored() {
        assert!(confirm_expected_matches_stored(&None, "1.2.0").is_err());
    }

    #[test]
    fn confirm_expected_refuses_a_version_different_from_the_stored_one() {
        let err = confirm_expected_matches_stored(&Some(available("1.3.0")), "1.2.0")
            .expect_err("must refuse");
        assert!(err.contains("1.3.0"));
        assert!(err.contains("1.2.0"));
    }

    #[test]
    fn confirm_expected_accepts_only_the_stored_version() {
        assert!(confirm_expected_matches_stored(&Some(available("1.2.0")), "1.2.0").is_ok());
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
    #[cfg(feature = "self-update")]
    fn describes_a_cancelled_install_without_the_raw_error_text() {
        let message = describe_error(&tauri_plugin_updater::Error::PackageInstallFailed);
        assert!(message.contains("password prompt"));
    }

    #[test]
    #[cfg(feature = "self-update")]
    fn describes_a_cancelled_authentication_prompt() {
        let message = describe_error(&tauri_plugin_updater::Error::AuthenticationFailed);
        assert!(message.contains("Authentication"));
    }

    #[test]
    #[cfg(feature = "self-update")]
    fn unrecognized_errors_fall_back_to_their_own_message() {
        let message = describe_error(&tauri_plugin_updater::Error::ReleaseNotFound);
        assert_eq!(
            message,
            tauri_plugin_updater::Error::ReleaseNotFound.to_string()
        );
    }

    #[test]
    fn dist_stamp_contents_matches_the_exact_format_build_rs_writes_by_hand() {
        assert_eq!(
            dist_stamp_contents("msstore", false),
            "LINH_BUILD_MARKER:dist=msstore;self_update=false\n"
        );
        assert_eq!(
            dist_stamp_contents("github", true),
            "LINH_BUILD_MARKER:dist=github;self_update=true\n"
        );
    }

    #[test]
    fn parse_dist_accepts_every_documented_value() {
        assert_eq!(parse_dist("github"), Some(Dist::Github));
        assert_eq!(parse_dist("msstore"), Some(Dist::Msstore));
        assert_eq!(parse_dist("mas"), Some(Dist::Mas));
        assert_eq!(parse_dist("flathub"), Some(Dist::Flathub));
        assert_eq!(parse_dist("copr"), Some(Dist::Copr));
        assert_eq!(parse_dist("ppa"), Some(Dist::Ppa));
    }

    #[test]
    fn parse_dist_rejects_anything_else() {
        assert_eq!(parse_dist(""), None);
        assert_eq!(parse_dist("Github"), None);
        assert_eq!(parse_dist("snap"), None);
    }

    #[test]
    fn only_github_is_unmanaged() {
        assert_eq!(Dist::Github.managed_by(), None);
    }

    #[test]
    fn every_other_dist_maps_to_its_own_managed_by() {
        assert_eq!(Dist::Msstore.managed_by(), Some(ManagedBy::MicrosoftStore));
        assert_eq!(Dist::Mas.managed_by(), Some(ManagedBy::MacAppStore));
        assert_eq!(Dist::Flathub.managed_by(), Some(ManagedBy::Flathub));
        assert_eq!(Dist::Copr.managed_by(), Some(ManagedBy::Copr));
        assert_eq!(Dist::Ppa.managed_by(), Some(ManagedBy::Ppa));
    }

    #[test]
    fn github_capability_is_untouched_by_the_override() {
        for base in [
            Capability::SelfInstall,
            Capability::DownloadOnly,
            Capability::None,
        ] {
            assert_eq!(capability_override_for_dist(Dist::Github, base), base);
        }
    }

    #[test]
    fn every_managed_dist_forces_capability_none_even_over_self_install() {
        for dist in [
            Dist::Msstore,
            Dist::Mas,
            Dist::Flathub,
            Dist::Copr,
            Dist::Ppa,
        ] {
            for base in [
                Capability::SelfInstall,
                Capability::DownloadOnly,
                Capability::None,
            ] {
                assert_eq!(
                    capability_override_for_dist(dist, base),
                    Capability::None,
                    "dist {dist:?} with base {base:?} must still end up None"
                );
            }
        }
    }

    #[test]
    fn scheduler_is_enabled_only_when_nothing_manages_updates() {
        assert!(scheduler_enabled(None));
    }

    #[test]
    fn scheduler_is_disabled_for_every_managed_by_value() {
        for managed in [
            ManagedBy::MicrosoftStore,
            ManagedBy::MacAppStore,
            ManagedBy::Flathub,
            ManagedBy::Copr,
            ManagedBy::Ppa,
        ] {
            assert!(!scheduler_enabled(Some(managed)));
        }
    }
}
