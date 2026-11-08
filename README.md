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
cd ui && pnpm install && cd ..
cargo tauri dev            # run the app
cargo tauri build --bundles rpm   # produce an installable .rpm
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

## License

Proprietary. Copyright (c) 2026 tkhieu. All rights reserved. See `LICENSE`.
