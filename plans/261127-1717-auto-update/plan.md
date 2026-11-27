# Auto-update and OTA updates for Linh

Status: APPROVED 2026-11-27. Owner decisions: scope (a)+(c); Linux in-place rpm/deb install with a password prompt; notify + "Update now", with no silent install. The signing key was generated on the owner's machine at `~/.tauri/linh-updater/` (private key, password and public key; mode 600; the owner keeps an offline backup). Repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` were set on 2026-11-27, and a test signature with the key passed.
Created: 2026-11-27
Base: `main` at v0.3.2 (`e37dce4`)

## Outcome

Installed copies of Linh find out about new releases by themselves and,
where the platform allows it, install them after one click. Presets
(Meet, Teams, Slack, Zoom) can be updated without shipping a new
binary.

## Scope

- **OTA (a), app self-update:** in scope. This uses `tauri-plugin-updater`
  (2.14.x; Linh's Tauri resolves to 2.12.5).
- **OTA (c), signed remote presets:** in scope, as the last phase.
- **OTA (b), hot-swapping the web UI without a new binary:** out of
  scope. `security.csp` is `null` (`src-tauri/tauri.conf.json:24`) and
  the UI has IPC access to config, browser and rule commands
  (`src-tauri/src/commands.rs`). A remote UI bundle would get all of
  that. Making it safe would cost as much as (a) and add attack surface.

## Non-goals

- Copr, PPA or apt/dnf repositories. They can come later; the plugin
  installs rpm/deb directly.
- Apple notarization and Windows code signing. Both are still
  owner-secret dependent (`docs/release.md`).
- Updating MSIX installs through the plugin, which the plugin does not
  support.

## Verified facts (two independent agents, 2026-11-27)

| Fact | Evidence |
|---|---|
| The plugin installs `.deb` (`dpkg -i`) and `.rpm` (`rpm -U`) in place, detecting the installed bundle type. It escalates via `pkexec`, then a graphical password tool, then `sudo`. A cancelled prompt returns `PackageInstallFailed`. | plugins-workspace `updater-v2.14.1`, `plugins/updater/src/updater.rs` L1197-1218, L1329-1415 |
| `createUpdaterArtifacts: true` only produces signed Linux artifacts for **AppImage**. For rpm/deb, CI must sign the files itself with `tauri signer sign` and publish the `.sig` contents. | v2.tauri.app/plugin/updater/#building |
| One `latest.json` can carry `linux-x86_64-rpm` and `linux-x86_64-deb`. The plugin tries the bundle-suffixed key first, then `linux-x86_64`. | updater.rs L3476-3509 |
| `darwin-universal` is not a standard key. Point both `darwin-aarch64` and `darwin-x86_64` at the universal `.app.tar.gz`. | updater.rs L2617-2621 |
| Plugin signatures do not make an unsigned macOS app pass Gatekeeper after an update. This is unverified, so macOS stays notify-only until Developer ID signing exists. | v2.tauri.app/distribute/sign/macos/ |
| The Windows plugin path handles NSIS and MSI, not MSIX. MSIX updates come from the Store or an `.appinstaller` file. | updater.rs L2244-2289, L3838-3861; Microsoft App Installer docs |
| Draft release assets are not publicly downloadable, and drafts cannot be "latest". Because Linh's release workflow always creates a draft, the update manifest must be published **after** the release is published. | GitHub REST releases docs |
| Linh is long-running (tray, single instance, `src-tauri/src/main.rs:134`), so periodic background checks fit. Settings has tabs `general \| browsers \| rules` (`ui/src/settings/SettingsApp.tsx:110,316-322`). | repo |

## Architecture

- **Endpoint:** `https://github.com/tkhieu/linh/releases/latest/download/latest.json`.
  "Latest" never points at a draft or a pre-release, so a release only
  reaches users once the owner publishes it.
- **Manifest builder:** a new workflow triggered on `release: published`.
  It downloads the release's `.sig` assets and writes a `latest.json`:
  - `linux-x86_64-rpm` → the `.rpm` file
  - `linux-x86_64-deb` → the `.deb` file
  - `windows-x86_64` → the NSIS `.exe`
  - `darwin-aarch64` and `darwin-x86_64` → the universal `.app.tar.gz`

  It uses the final `releases/download/<tag>/` URLs, uploads
  `latest.json` to that release, and fails if any `.sig` is missing.
- **Signing:**
  - The owner generates one minisign key pair (`tauri signer generate`).
  - The private key and its password become repo secrets.
  - The public key goes into `plugins.updater.pubkey`.
  - `release.yml` signs the rpm, deb, NSIS exe and `.app.tar.gz` and
    uploads the `.sig` files next to them.
  - If the key is lost, existing installs can never update again. The
    owner keeps an offline backup.
- **Runtime:**
  - Check on startup (after a short delay), then every 24 h, with no
    retry storm on network failure.
  - A found update shows a notification and an "Updates" section in
    Settings (current version, available version, notes, "Update now").
  - Installing downloads the update, verifies its signature (the plugin
    does this), installs it, and relaunches.
  - Settings get a toggle "Check for updates automatically" (default
    on).
  - Silent auto-install is not offered in the first version.
- **Per-install capability:** a `can_self_update` report from the
  backend decides what the UI offers:
  - **rpm/deb, NSIS:** "Update now".
  - **macOS (unsigned):** "Download" (opens the release page).
  - **MSIX:** hidden, because the Store or `.appinstaller` handles it.
  - **Dev/unknown builds:** check only.
- **Remote presets (OTA c):**
  - A `presets.json` is signed with the same key and published with
    each release, or on its own channel.
  - Linh fetches it daily, verifies the minisign signature in Rust, and
    validates it against the preset schema.
  - It replaces only data: patterns and app-target metadata. Nothing in
    it is ever executed or turned into argv beyond what the compiled
    preset logic already allows.
  - The compiled-in presets remain the fallback.

## Phases (one PR each, cross-reviewed)

| # | PR | Files (expected) | Depends on |
|---|---|---|---|
| 1 | **Signing in CI.** Add `pubkey` and updater config to `tauri.conf.json` (plugin not yet wired), and a sign step in `release.yml` producing `.sig` for rpm, deb, exe and `.app.tar.gz` (adding the macOS `app` bundle). Prove it with a `workflow_dispatch` dry run. Document key handling in `docs/release.md`. | `.github/workflows/release.yml`, `src-tauri/tauri.conf.json`, `docs/release.md` | Owner adds the key secrets |
| 2 | **Manifest workflow.** `release: published` builds and uploads `latest.json`, using a small script under `scripts/` with unit tests and a fixture release. | `.github/workflows/update-manifest.yml`, `scripts/build-update-manifest.py` + tests | 1 |
| 3 | **Updater runtime.** Add the plugin dependency and capability, the background check scheduler, the `can_self_update` detection, the notification and the install command. The Rust logic sits behind a testable seam. | `src-tauri/Cargo.toml`, `src-tauri/src/updates.rs`, `main.rs`, `capabilities/*.json` | 1 |
| 4 | **Settings "Updates" UI.** Status, notes, "Update now", the auto-check toggle (a config field with a migration default), and Vitest coverage. | `ui/src/settings/*`, `ui/src/api.ts`, `crates/linh-core/src/config.rs` | 3 |
| 5 | **End-to-end update test in CI.** A local static server, a throwaway key, a fixture `latest.json`, and an install from vN to vN+1 on Linux (rpm in a fedora container, deb in ubuntu) and Windows NSIS. | `.github/workflows/*`, `tests/` | 2, 3 |
| 6 | **Signed remote presets (OTA c).** Signed `presets.json` publishing, then fetch, verify, validate and fall back. | `crates/linh-core/src/presets.rs`, `src-tauri/src/…`, workflow step | 1, 3 |

Phases 2 and 3 can run in parallel after phase 1. Each PR touches CI or
config, so each one needs a human merge.

## Validation

- The unit tests listed per phase, plus the usual gates (`cargo fmt`,
  `cargo clippy -D warnings`, `cargo test --workspace`, the UI lint,
  typecheck and test, and the rpm build).
- A release dry run after phases 1 and 2.
- The first real check is v0.4.0 → v0.4.1 on the owner's Fedora machine:
  update from Settings, confirm the polkit prompt, relaunch, and confirm
  the new version.

## Risks

- **Key loss** strands every install. The owner keeps an offline backup;
  rotation means shipping a release that contains the new public key
  before switching.
- **Linux privilege prompt:** without a polkit agent, `sudo` falls back to
  a terminal. The plugin returns `PackageInstallFailed`, and Linh then
  offers "Download" instead.
- **Update order:** the first release that contains the updater (v0.4.0)
  has to be installed by hand. Updates work from then on.
- **macOS:** self-install waits for Developer ID signing and
  notarization.
- **Remote presets** are only as safe as the schema validation. They are
  data-only by design.

## Open decisions (owner)

1. Is OTA scope (a) + (c) right, with (b) dropped?
2. Linux: in-place rpm/deb install with a password prompt (default), or
   notify-and-download only?
3. Is "notify + Update now" the right behaviour, with no silent
   auto-install?
4. Will you generate the signing key and add the two secrets
   (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`)?
