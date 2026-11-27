//! WebKitGTK-specific memory tuning (tkhieu/linh#15 step 2). Linux only:
//! other OSes get no-op stubs so `main.rs` can call these unconditionally.
//!
//! Each setting here was measured independently (isolated HOME/XDG dirs,
//! `dbus-run-session`, median of 3 fresh-idle snapshots plus a
//! picker-shown snapshot, release build) and kept only because it
//! lowered tree PSS with no observed downside; see this branch's PR for
//! the numbers. Three candidates were tried and dropped:
//!
//! - `webkit_website_data_manager_set_memory_pressure_settings()` (tunes
//!   the `WebKitNetworkProcess`) must run before the first
//!   `WebKitWebsiteDataManager` is created, which — since Tauri builds its
//!   `WebContext` before `Builder::setup` ever runs — forced calling it
//!   from the very top of `main`, before GTK is initialized. Working
//!   around that with an explicit early `gtk::init()` made startup
//!   regress to ~35s (apparently from GTK's main-context acquire-and-leak
//!   during `init()`, see <https://github.com/gtk-rs/gtk-rs-core/issues/186>,
//!   interacting badly with Tauri's own later init on the same context) —
//!   not worth it for a few MB on a process that's already small at idle.
//! - `WEBKIT_DISABLE_COMPOSITING_MODE=1` and `WEBKIT_DISABLE_DMABUF_RENDERER=1`
//!   were tried together as a way to cut `WebKitWebProcess` memory by
//!   forcing WebKitGTK 2.54's Skia backend onto its CPU rasterizer
//!   instead of its GPU-accelerated one. On WebKitGTK 2.54.2 the two
//!   vars aren't independent switches: `AcceleratedBackingStore::rendererBufferTransportMode()`
//!   treats `WEBKIT_DISABLE_DMABUF_RENDERER=1` as "no renderer buffer
//!   transport available at all" and returns an empty set without even
//!   probing EGL
//!   (<https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.2/Source/WebKit/UIProcess/gtk/AcceleratedBackingStore.cpp#L86-L88>),
//!   which makes `AcceleratedBackingStore::canUseHardwareAcceleration()`
//!   report `false`
//!   (<https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.2/Source/WebKit/UIProcess/gtk/AcceleratedBackingStore.cpp#L139-L144>).
//!   `HardwareAccelerationManager`'s constructor checks that first and,
//!   if hardware acceleration can't be used at all, forces
//!   `m_acceleratedCompositingModeEnabled = false` unconditionally —
//!   `WEBKIT_DISABLE_COMPOSITING_MODE` is only even read in the `else`
//!   branch, once hardware acceleration is already available
//!   (<https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.2/Source/WebKit/UIProcess/gtk/HardwareAccelerationManager.cpp#L42-L47>).
//!   So on 2.54.2, *either* var disables accelerated compositing
//!   outright; there's no combination that disables only DMA-BUF import
//!   while leaving compositing itself on. Two separate owner tests
//!   against the owner's screen recording of v0.3.1 on Fedora 44/GNOME
//!   50 Wayland (mutter 50.6) confirmed the picker breaks without
//!   accelerated compositing either way: with
//!   `WEBKIT_DISABLE_COMPOSITING_MODE=1` set, and again with
//!   `WEBKIT_DISABLE_COMPOSITING_MODE=0`/`WEBKIT_DISABLE_DMABUF_RENDERER=1`
//!   — both left the transparent, undecorated link picker window
//!   (`ui/src/prompt/PromptApp.tsx`) breaking up over several seconds
//!   while the user moved between rows: header/footer vanishing, the
//!   row list splitting into separate floating boxes with the desktop
//!   visible between them. The passing test set both vars to `0`
//!   explicitly; that's equivalent to leaving both unset, not a third
//!   state — `rendererBufferTransportMode()`'s DMA-BUF check only skips
//!   probing EGL when the var is present *and* not equal to `"0"`
//!   (`disableDMABuf && g_strcmp0(disableDMABuf, "0")` at the line above),
//!   so `"0"` and absent take the same path, and
//!   `m_acceleratedCompositingModeEnabled`/`m_canUseHardwareAcceleration`
//!   both default to `true` in
//!   [`HardwareAccelerationManager.h`](https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.2/Source/WebKit/UIProcess/gtk/HardwareAccelerationManager.h#L44-L45),
//!   the same value an explicit `WEBKIT_DISABLE_COMPOSITING_MODE=0`
//!   produces. [`configure_before_tauri`] therefore just leaves both
//!   vars unset — Linh no longer manages them in either direction — and
//!   the ~30-40MB tkhieu/linh#25 measured from forcing them is withdrawn
//!   entirely.
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

#[cfg(target_os = "linux")]
use webkit2gtk::{CacheModel, SettingsExt, WebContextExt, WebViewExt};

/// No longer sets anything. This used to force the WebKitGTK
/// renderer-selection env vars `WEBKIT_DISABLE_COMPOSITING_MODE` and
/// `WEBKIT_DISABLE_DMABUF_RENDERER` before Tauri could launch the first
/// `WebKitWebProcess` (the only point in Linh's startup where either var
/// could still take effect); both turned out to corrupt the link
/// picker's repaints on GNOME 50 Wayland with WebKitGTK 2.54.2 — see
/// this module's top-level doc comment for the two owner tests that
/// showed it — so neither is set any more and this is kept only as the
/// designated hook for that same timing requirement, should a future
/// setting need it.
pub fn configure_before_tauri() {}

/// Tunes the WebKitGTK behavior that's only reachable once a webview
/// exists. `settings` is shared by every `WebKitWebView` created with
/// `with_related_view` from this one (confirmed in the WebKitGTK docs
/// for `webkit_web_view_new_with_related_view`: the new view "will also
/// have the same WebKitSettings"), so this tuning is visible to
/// `settings_window`'s lazily created view too — *but* constructing that
/// second view flips the shared page-cache setting back on (see below),
/// so `settings_window` still needs its own re-apply call after it's
/// built; only the cache-model tuning here is truly one-and-done.
///
/// - Cache model `DocumentViewer`: Linh never browses between documents
///   (`prompt`/`settings` each show exactly one local page), so the
///   default `WebBrowser` model's large object/page caches buy nothing.
///   Measured to save ~16MB idle and ~33MB with the picker shown.
/// - Page cache off: it only pays off across in-page navigations, which
///   Linh's UI never performs. Measured to save another ~10MB idle,
///   ~11MB with the picker shown — all of it attributable to this one
///   setting. A second knob, disabling the legacy offline-application
///   cache, was tried alongside it but turned out to be a no-op in the
///   installed WebKitGTK 2.54.2: its setter doesn't do anything and its
///   getter always reports `false` regardless (confirmed against
///   upstream's `WebKitSettings.cpp`,
///   <https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.2/Source/WebKit/UIProcess/API/glib/WebKitSettings.cpp#L1814-L1829>),
///   so it was dropped — the saving above is the page cache's alone.
///
/// wry 0.57.1 unconditionally re-enables the page cache while
/// constructing *every* `WebKitWebView` (see
/// `settings_window::reapply_page_cache_off`'s doc comment for why that
/// also requires re-applying this setting after Settings is built).
#[cfg(target_os = "linux")]
pub fn tune_webview(webview: webkit2gtk::WebView) {
    if let Some(context) = webview.context() {
        context.set_cache_model(CacheModel::DocumentViewer);
    }
    if let Some(settings) = webview.settings() {
        settings.set_enable_page_cache(false);
    }
}
