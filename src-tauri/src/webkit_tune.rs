//! WebKitGTK-specific memory tuning (tkhieu/linh#15 step 2). Linux only:
//! other OSes get no-op stubs so `main.rs` can call these unconditionally.
//!
//! Each setting here was measured independently (isolated HOME/XDG dirs,
//! `dbus-run-session`, median of 3 fresh-idle snapshots plus a
//! picker-shown snapshot, release build) and kept only because it
//! lowered tree PSS with no observed downside; see this branch's PR for
//! the numbers. One candidate was tried and dropped:
//! `webkit_website_data_manager_set_memory_pressure_settings()` (tunes
//! the `WebKitNetworkProcess`) must run before the first
//! `WebKitWebsiteDataManager` is created, which — since Tauri builds its
//! `WebContext` before `Builder::setup` ever runs — forced calling it
//! from the very top of `main`, before GTK is initialized. Working
//! around that with an explicit early `gtk::init()` made startup
//! regress to ~35s (apparently from GTK's main-context acquire-and-leak
//! during `init()`, see <https://github.com/gtk-rs/gtk-rs-core/issues/186>,
//! interacting badly with Tauri's own later init on the same context) —
//! not worth it for a few MB on a process that's already small at idle.
//!
//! What's left, confirmed against `/usr/share/gir-1.0/WebKit2-4.1.gir`
//! (the installed WebKitGTK 2.54.2's own API docs):
//!
//! - `webkit_web_context_set_cache_model()` and `WebKitSettings`
//!   properties are *not* construct-only, so [`tune_webview`] can set
//!   them once a webview exists — reachable from `main.rs`'s `setup`
//!   via `WebviewWindow::with_webview`, which is the earliest point
//!   application code can reach into WebKitGTK at all (Tauri builds the
//!   `prompt` window, and with it its `WebContext`, before `setup` runs,
//!   with no hook to intervene earlier).
//! - `WEBKIT_DISABLE_COMPOSITING_MODE`/`WEBKIT_DISABLE_DMABUF_RENDERER`
//!   are read once at `WebKitWebProcess` startup, so they must be set
//!   before Tauri launches that process — i.e. from [`configure_before_tauri`].

#[cfg(target_os = "linux")]
use webkit2gtk::{CacheModel, SettingsExt, WebContextExt, WebViewExt};

/// Must run before Tauri's `Builder::run` launches the first
/// `WebKitWebProcess` — i.e. at the top of `main`, before
/// `tauri::Builder::default()...run()`.
///
/// Disables WebKit's GPU-accelerated (DMA-BUF/EGL) compositing in favor
/// of software (Cairo) rendering: measured to save ~30MB idle and ~40MB
/// with the picker shown, by far the largest single saving tried here,
/// avoiding the GL/EGL/Mesa driver stack entirely for an app that never
/// needs GPU-accelerated video or 3D. `WEBKIT_DISABLE_COMPOSITING_MODE`
/// is the flag older WebKitGTK versions documented;
/// `WEBKIT_DISABLE_DMABUF_RENDERER` is 2.54's actual renderer-selection
/// switch, confirmed present in the installed `libwebkit2gtk-4.1.so` by
/// `strings`. Both are set for forward/backward compatibility. Linh's
/// own CSS (`ui/src/styles.css`) uses only borders and border-radius —
/// no `transform`, `filter`, `backdrop-filter`, or `animation` — which
/// the software path renders identically, and window transparency/
/// rounding is a GTK/window-manager property independent of this flag,
/// not WebKit's internal layer compositing. Page-load timing and
/// process lifecycle were confirmed unaffected (see the PR); exact
/// pixel-level rendering (fonts, corners, flicker) was not screenshotted
/// and still needs a human look before relying on it further.
#[cfg(target_os = "linux")]
pub fn configure_before_tauri() {
    // SAFETY: called at the very top of `main`, before any thread is
    // spawned or GTK/WebKit touches the environment, so nothing else can
    // race this write.
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn configure_before_tauri() {}

/// Tunes the WebKitGTK behavior that's only reachable once a webview
/// exists. `settings` is shared by every `WebKitWebView` created with
/// `with_related_view` from this one (confirmed in the WebKitGTK docs
/// for `webkit_web_view_new_with_related_view`: the new view "will also
/// have the same WebKitSettings"), so tuning `prompt`'s view here also
/// covers `settings_window`'s lazily created view — it doesn't need its
/// own call.
///
/// - Cache model `DocumentViewer`: Linh never browses between documents
///   (`prompt`/`settings` each show exactly one local page), so the
///   default `WebBrowser` model's large object/page caches buy nothing.
///   Measured to save ~16MB idle and ~33MB with the picker shown.
/// - Page cache and the legacy offline-application-cache off: both only
///   pay off across in-page navigations, which Linh's UI never performs.
///   Measured to save another ~10MB idle, ~11MB with the picker shown.
#[cfg(target_os = "linux")]
pub fn tune_webview(webview: webkit2gtk::WebView) {
    if let Some(context) = webview.context() {
        context.set_cache_model(CacheModel::DocumentViewer);
    }
    if let Some(settings) = webview.settings() {
        settings.set_enable_page_cache(false);
        settings.set_enable_offline_web_application_cache(false);
    }
}
