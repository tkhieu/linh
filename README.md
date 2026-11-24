# Linh

A Velja-style browser picker: registers as the default browser and routes
each URL to the right browser, profile, or private window based on rules
you configure. This repo currently targets Fedora (GNOME/Wayland).

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
also always logs one cheap, synchronous line once both the real
`settings` and `prompt` windows have finished their first page load:
`webview-create startup ready (process_start->page_load_finished):
settings=<ms> prompt=<ms>`. If one of them never finishes loading, that
line is simply never logged.

`--out FILE` is meant for a path you own: it refuses to write through an
existing symlink or non-regular file, but it is not hardened against a
concurrent attacker racing a symlink into place at that exact path.

## License

Proprietary. Copyright (c) 2026 tkhieu. All rights reserved. See `LICENSE`.
