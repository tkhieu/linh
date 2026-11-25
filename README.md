# Linh

A Velja-style browser picker: registers as the default browser and routes
each URL to the right browser, profile, or private window based on rules
you configure. Native builds target Fedora, Ubuntu 24.04, and Windows 10
version 1809 or later.

Routes are checked in order before the prompt mode applies. The first matching
route opens its configured destination, including an app or the picker. When no
route matches, `prompt_mode = "always"` opens the picker and
`prompt_mode = "never"` opens the primary browser.

## Repo layout

- `crates/linh-core` — pure-Rust routing logic (config, URL matching,
  routing decisions, browser model, launch-arg builder). No UI/Tauri deps.
- `src-tauri` — the Tauri v2 desktop app (Rust backend + webview windows).
- `ui` — the Settings and Prompt windows (Vite + React + TypeScript).

## Build on Fedora 44

Install the system dependencies Tauri needs to build and run:

```sh
sudo dnf install -y \
  webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel \
  librsvg2-devel libxdo-devel openssl-devel rpm-build
```

Install Rust (via [rustup](https://rustup.rs)) and Node.js + pnpm, then:

```sh
cargo install tauri-cli --version '^2' --locked   # once
pnpm --dir ui install
cd src-tauri                      # run the Tauri CLI from here (as CI does)
cargo tauri dev                   # run the app
cargo tauri build --bundles rpm   # produce target/release/bundle/rpm/Linh-*.rpm
```

Install on Ubuntu 24.04: download the CI `.deb` artifact and run `sudo apt install ./linh_*.deb`.

## Build on Windows

Install Rust, Node.js, and pnpm. From the repository root run
`pnpm --dir ui install`; then run `cargo tauri build --bundles nsis`
from `src-tauri`. The Windows CI job
uploads an unsigned NSIS installer. It installs for the current user.

Linh finds browsers registered under Windows' StartMenuInternet keys and
uses each browser's HTTPS ProgID open command when available. Chrome, Edge, Brave, Vivaldi,
and Firefox support private launches; Opera uses `--private`. Chromium
profiles come from each browser's `Local State` file. Teams and Zoom use
their registered `msteams:` and `zoommtg:` protocol handlers.

To make Linh the default browser, click **Open Windows Default Apps settings**
in Linh. Linh registers its HTTP/HTTPS capabilities for the current user
and opens Windows Settings. Choose Linh for both HTTP and HTTPS there;
Windows 10/11 requires this user selection. Return to Linh to confirm
the default status. Browser icons are extracted from registered EXE/DLL
resources as PNG data URLs. Windows notification errors are recorded in
the Linh log under `%LOCALAPPDATA%\Linh\linh.log`.
At startup, Linh repairs an existing per-user URL registration if it points
to an older executable. Uninstall removes Linh's per-user ProgID,
capabilities, and RegisteredApplications value.

## Tests and checks

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

cd ui
pnpm lint
pnpm typecheck
pnpm test
```

## Profiling memory

`scripts/mem-profile.sh` reports PSS/RSS/USS/SwapPss for Linh's process
tree (`linh` + its `WebKitWebProcess`/`WebKitNetworkProcess`/GPU
descendants), read from each process's `/proc/<pid>/smaps_rollup`:

```sh
scripts/mem-profile.sh snapshot idle                  # one labeled snapshot
scripts/mem-profile.sh snapshot idle --out mem.tsv     # + append a TSV row
scripts/mem-profile.sh guided                          # walk idle/settings/prompt/closed, with a summary
scripts/mem-profile.sh snapshot idle --exe target/debug/linh   # profile a dev build instead of /usr/bin/linh
```

**PSS is the number to compare across snapshots.** `RSS` counts a page in
full for every process mapping it, so summing RSS across Linh's
processes double-counts WebKit's shared library/cache pages — the script
never does that, and shows RSS per-process only. `PSS` (proportional set
size) instead divides each shared page's cost evenly across the
processes mapping it, so the tree's PSS total is the right number for
"how much real memory does this idle instance cost." `USS` (unique set
size: pages private to one process) is the floor you'd recover if every
other process using those shared pages also exited. `SwapPss` is PSS's
swapped-out counterpart — on a long-running, swapped instance, PSS +
SwapPss together are the full cost, not PSS alone.

To additionally measure the cold cost of *creating* a webview (used to
decide whether windows like Settings can be created on demand instead of
at startup), set `LINH_PROFILE_WEBVIEW=N` (1–50) before launching Linh.
It creates and destroys `N` throwaway hidden webviews after startup and
logs each sample plus min/median/max to `~/.local/state/linh/linh.log`
under a `webview-create` prefix. Unset (the default), this does nothing
extra at all — no extra thread, no extra window, no extra timer. Linh
also always logs one cheap, synchronous line per window once it finishes
its first page load: `prompt` (created at startup) logs
`webview-create startup ready (process_start->page_load_finished):
prompt=<ms>`; `settings` is created lazily on first demand (see below)
and logs `webview-create settings first-open ready
(create_requested->page_load_finished): settings=<ms>` instead, timed
from when it was first requested rather than from process start. If a
window never finishes loading, its line is simply never logged.

## Settings window

`settings` is not created at startup — only `prompt` is. Settings is
built the first time it's requested (tray "Settings…", or launching Linh
again with no URL to route) and, on Linux, related to `prompt`'s webview
(`with_related_view`) so it shares `prompt`'s `WebKitWebProcess` instead
of spawning a second one. Closing it hides the window rather than
destroying it, so reopening is instant and no extra process accumulates;
`Window::destroy()` is never used for it, since wry 0.57.1 keeps a
window's `WebKitWebProcess` alive past `destroy()` anyway (a reference
cycle in its IPC handler), so destroying it would free nothing while
losing the instant-reopen behavior.

`--out FILE` is meant for a path you own: it refuses to write through an
existing symlink or non-regular file, but it is not hardened against a
concurrent attacker racing a symlink into place at that exact path.

## WebKitGTK memory tuning

`src-tauri/src/webkit_tune.rs` (Linux only) applies the WebKitGTK-level
settings that measurably lower idle and picker-shown PSS for a
single-purpose app that never browses between documents:

- `webkit_web_context_set_cache_model(DOCUMENT_VIEWER)` instead of the
  default `WEB_BROWSER` model's large object/page caches.
- `WebKitSettings`' page cache disabled — it only pays off across
  in-page navigations Linh never performs. A second knob, the legacy
  offline-application-cache, was tried alongside it but turned out to be
  a no-op in the installed WebKitGTK 2.54.2 (setter does nothing, getter
  always reports `false`), so it was dropped; the measured saving
  belongs to the page cache alone. Because `with_related_view` makes
  `prompt` and `settings` share one `WebKitSettings`, and wry
  unconditionally re-enables the page cache while constructing each new
  `WebKitWebView`, this setting is re-applied right after the Settings
  window is built — otherwise opening Settings once would silently turn
  the page cache back on for both windows.
- `WEBKIT_DISABLE_COMPOSITING_MODE`/`WEBKIT_DISABLE_DMABUF_RENDERER` set
  before Tauri launches its first `WebKitWebProcess`, forcing WebKitGTK's
  Skia backend to rasterize on the CPU instead of using GPU-accelerated
  (DMA-BUF/EGL) compositing — WebKitGTK 2.54 removed the old Cairo 2D
  backend entirely, so this is a GPU/CPU split within Skia, not a switch
  to Cairo. Linh's UI uses only borders and `border-radius`, nothing
  that path depends on. GPU compositing is also what makes scrolling
  smooth, so that's the one thing the owner still needs to check
  visually on Wayland. These two vars are process-wide, so every
  `Command` Linh runs on Linux chains
  `.strip_linh_webkit_env()` (the `StripLinhWebkitEnv` extension trait in
  `src-tauri/src/platform/launcher.rs`) right after `Command::new`, which
  removes only the vars `configure_before_tauri` itself introduced (a
  value the user already had set in their own environment before Linh
  started is left exactly as it was). This covers every child process
  Linh spawns: `spawn_detached` (browsers and app targets),
  `xdg-settings get`/`xdg-settings set`/`xdg-mime default` in
  `platform/linux/default_browser.rs`, and `notify-send` in
  `platform/linux/notify.rs` — so a launched WebKitGTK app (e.g. GNOME
  Web) never inherits Linh's own rendering policy.

A fourth candidate — tuning the `WebKitNetworkProcess`'s own memory-
pressure thresholds via
`webkit_website_data_manager_set_memory_pressure_settings()` — was tried
and dropped: it must run before Tauri builds its first
`WebKitWebsiteDataManager`, which forced an explicit `gtk::init()` call
before GTK would otherwise be initialized, and that made startup regress
to tens of seconds. Not worth it for a few MB on an already-small
process.

## License

Copyright (C) 2026 tkhieu. Linh is licensed under
[GPL-3.0-only](LICENSE). Bundled IBM Plex and Martian Mono fonts are licensed
under the SIL Open Font License 1.1; see [third-party notices](THIRD_PARTY_NOTICES.md).
