# Releasing Linh

Linh's public GitHub releases use GPL-3.0-only. GitHub's Windows NSIS download is
unsigned; the Microsoft Store is the signed Windows channel. The Store MSIX
artifact is for Partner Center submission, not direct installation.

## Release steps

1. Set the same `MAJOR.MINOR.PATCH` version in `Cargo.toml` under
   `[workspace.package]`, `src-tauri/tauri.conf.json`, and `ui/package.json`.
   Run `python3 scripts/check-release-version.py`. The workflow rejects a tag
   unless all three versions equal the `vMAJOR.MINOR.PATCH` tag. Pre-release
   and build suffixes (`v1.2.3-beta`, `v1.2.3+build`) are rejected too: MSIX
   package versions must be a plain numeric `Major.Minor.Patch`.
2. Run **Release** with `workflow_dispatch` and `dry_run=true` on a branch
   after the workflow is available on the default branch. Inspect all four
   build jobs and the complete `linh-release-assets` artifact, including
   `SHA256SUMS`. Dry runs create no GitHub Release. The initial PR
   can use a temporary pull request trigger for verification before merge.
   `workflow_dispatch` also takes `skip_signing` (default `false`), honored
   only together with `dry_run=true`: it forces an unsigned build even
   though the signing secrets are configured, so the no-secrets path (the
   one a fork without the secrets actually takes) gets exercised on demand
   without having to remove the repository's own secrets to test it.
3. Merge the change, then push the `vMAJOR.MINOR.PATCH` tag. The tag build
   creates a **draft** GitHub Release with `.rpm`, `.deb`, unsigned NSIS `.exe`,
   Store `.msix`, universal macOS `.dmg`, the macOS updater archive
   (`linh.app.tar.gz`), a `.sig` for each of the rpm, deb, NSIS exe, and
   `.app.tar.gz` (four in total; the msix and dmg are never signed),
   `SHA256SUMS`, and GitHub-generated notes from changes since the previous
   tag. Check the files and notes, then publish the draft with an owner
   personal access token using `gh release edit vMAJOR.MINOR.PATCH --draft=false`.
   Publishing with `GITHUB_TOKEN` does not trigger the Update manifest workflow.
4. Submit the `.msix` separately in Partner Center and manually test Windows
   default-browser selection and URL forwarding on the Store-installed app.
   Also test the macOS DMG on a Mac before publishing the GitHub draft.

## Update manifest

Publishing a stable draft release with the owner's personal token triggers
**Update manifest**. It downloads that release's signed RPM, DEB, NSIS setup
EXE, and universal macOS app archive, then uploads `latest.json` to the same
release. It refuses drafts, prereleases, missing or empty artifacts and
signatures, and version mismatches. v0.3.2 and earlier have no published `.sig`
assets, so the workflow cannot build a manifest for those releases.

To backfill or repair a published, signed release, run **Update manifest** with
`workflow_dispatch` and `tag=vX.Y.Z`. The workflow replaces that release's
`latest.json`. For the current latest release, check that
`https://github.com/tkhieu/linh/releases/latest/download/latest.json` returns
JSON with the expected version and platform entries. The workflow checks this
URL after upload; when backfilling an older release it warns that the target
is not latest and skips the URL check.

## GitHub settings

| Kind | Name | Purpose |
| --- | --- | --- |
| Secret | `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12`. |
| Secret | `APPLE_CERTIFICATE_PASSWORD` | Password for the exported certificate. |
| Secret | `APPLE_SIGNING_IDENTITY` | Developer ID Application identity for Tauri. |
| Secret | `APPLE_ID` | Apple account email for notarization. |
| Secret | `APPLE_PASSWORD` | Apple app-specific password for notarization. |
| Secret | `APPLE_TEAM_ID` | Apple Developer Team ID for notarization. |
| Variable | `MSIX_IDENTITY_NAME` | Identity Name from Partner Center's Product identity page. |
| Variable | `MSIX_PUBLISHER` | Exact Publisher distinguished name from that page. |
| Variable | `MSIX_PUBLISHER_DISPLAY_NAME` | Exact PublisherDisplayName from that page. |

When any Apple secret is absent, the macOS job builds without Developer ID
signing or notarization and emits a notice; an unsigned DMG must be handled
accordingly during manual testing. When all six are present, Tauri uses its
documented Apple environment variables to sign and notarize. Never place
secrets in repository variables or in the manifest. The MSIX variables default
to conspicuous `Placeholder` values for CI verification; replace **all three**
with the exact Partner Center values before submitting a Store package.

## Store build flavour (`LINH_DIST`)

Every build has one distribution channel, `LINH_DIST`, baked in at compile
time by `src-tauri/build.rs` (default `github` when the variable is unset).
It is read once, by `build.rs`, not at runtime: `cargo:rustc-env` embeds the
validated value into the binary, and an unrecognized value fails the build
immediately with the list of accepted ones. The six values are `github`,
`msstore`, `mas`, `flathub`, `copr`, `ppa`.

For every value except `github`, Settings → Updates shows "Updates for this
version come from …" instead of any checking/installing UI, and the
background scheduler never starts (`updates::scheduler_enabled`) — so a
store or distro build never contacts the update endpoint at all.

Microsoft Store policy and Apple's App Store guideline 2.5.2 additionally
forbid shipping a self-updating code path at all, not just disabling it at
runtime. `tauri-plugin-updater` is gated behind the `self-update` Cargo
feature (default **on**), so `msstore`/`mas` builds must turn it off
entirely — the dependency, the plugin registration in `main.rs`, and every
function in `updates.rs` that calls it are all compiled out. `build.rs`
enforces the combination: it fails the build if `LINH_DIST` is `msstore` or
`mas` while `self-update` is still enabled, so this can't be gotten wrong
silently.

`cargo tauri build` only forwards flags it recognizes itself (`--bundles`,
`--features`, `--target`, …); anything meant for the underlying `cargo`
invocation — `--no-default-features`, `--target-dir` — must come after a
literal `--`, or it's rejected outright:

```
$ cargo tauri build --no-default-features
error: unexpected argument '--no-default-features' found
  tip: to pass '--no-default-features' as a value, use '-- --no-default-features'
```

(`cargo tauri build --help`, tauri-cli 2.12.4: *"\[ARGS\]... Command line
arguments passed to the runner. Use `--` to explicitly mark the start of
the arguments"*.) Every `cargo`/`cargo tauri` command below was actually
run against this repo (on Linux) to confirm it parses and builds — the
`msstore`/`mas` `--bundles`/packaging steps are Windows/macOS-only (Linux's
`tauri-cli` only recognizes `deb`/`rpm`/`appimage` for `--bundles`, and
`package-msix.ps1` needs the Windows SDK), so those two are shown in the
shell syntax `release.yml` actually uses them in (bash on Linux runners,
PowerShell on the Windows runner — see its "Build Windows Store binary"
and "Package unsigned Windows Store MSIX" steps) rather than re-verified
from this checkout; the `--` requirement itself is a `cargo tauri`
behavior common to every platform, and that part was verified here.

```sh
# GitHub (default): the in-app updater is this install's update path.
cargo tauri build --bundles rpm   # or deb/nsis/dmg, per platform

# Microsoft Store (Windows, PowerShell): `self-update` must be off, and
# the exe that ends up in the MSIX must not be the same one the NSIS
# installer just built. Build it into its own --target-dir, then package
# it — package-msix.ps1 itself verifies the exe by searching its bytes
# for its build marker before packaging it (see below), so this doesn't
# have to be done by hand.
$env:LINH_DIST = 'msstore'
cargo tauri build --no-bundle -- --no-default-features --target-dir ../target/msstore
$env:LINH_STORE_EXE_PATH = 'target/msstore/release/linh.exe'
$env:LINH_VERSION = '0.4.1'
./scripts/package-msix.ps1

# Mac App Store: same self-update-off requirement, run on macOS. The
# `app` bundle target (the unsigned .app the sandboxing/signing work in a
# later phase builds on) only exists in macOS's own tauri-cli.
LINH_DIST=mas cargo tauri build --bundles app -- --no-default-features

# Flathub / Copr / PPA: `self-update` stays on (no store policy forbids
# it here), but it never runs — the distro/Flathub updates these installs,
# not Linh. Each reuses the existing rpm/deb bundle; the packaging work
# specific to each channel (the Copr spec, the PPA source package, the
# Flathub manifest) is tracked separately in the store-submissions plan
# (phases 4, 5, 7) and isn't built by this command on its own yet.
LINH_DIST=copr cargo tauri build --bundles rpm
LINH_DIST=ppa cargo tauri build --bundles deb
LINH_DIST=flathub cargo tauri build --bundles rpm
```

Every built binary embeds its own compiled-in channel directly in its
data section as plain bytes: `updates::BUILD_MARKER`, e.g.
`LINH_BUILD_MARKER:dist=msstore;self_update=false`. This is checked by
searching the built *file*, never by running it — `linh` is a
GUI-subsystem binary (`windows_subsystem = "windows"`), which doesn't
have usable stdout when spawned non-interactively from a script (an
earlier version of this ran `linh.exe --print-dist` and crashed in CI
with "failed printing to stdout: The pipe is being closed"), and
starting the real binary at all would run its single-instance/plugin
startup code for no reason. `scripts/package-msix.ps1` requires
`LINH_STORE_EXE_PATH` (no default/guessed path), reads that exe's raw
bytes, and refuses to package anything that doesn't contain exactly
`LINH_BUILD_MARKER:dist=msstore;self_update=false` — so pointing it at
the wrong exe (e.g. the NSIS build's own `target/release/linh.exe`)
fails the packaging step instead of silently shipping the self-updater
in a Store submission.

CI's `build` job runs `cargo check` and `cargo clippy` a second time with
`LINH_DIST=msstore` and `--no-default-features` on every push, so the store
configuration can't silently rot even though no workflow builds and submits
the actual Store packages yet.

## Store MSIX

Tauri builds the unsigned NSIS installer using its existing Windows build
command. The release job stages the same `target/release/linh.exe`, GPL license,
MSIX manifest, and logo assets, then uses the Windows SDK `MakePri.exe` and
`MakeAppx.exe` to produce an unsigned x64 `.msix`. This keeps the packaging
step small and uses Microsoft's own package validator. The manifest targets
Windows 10 1809 or later, declares `runFullTrust`, HTTP/HTTPS protocol and
HTML file associations, and a `linh.exe` execution alias. The MSIX version is
`MAJOR.MINOR.PATCH.0`; increase the app version for each Store update.

Reserve the Linh app name in Partner Center, copy its Product identity values
into the repository variables above, run a new dry run or tag build, and
upload the `.msix` from the workflow artifact or draft release to the app's
**Packages** page. Partner Center accepts `.msix` (though `.msixupload` is
recommended when Visual Studio produces it), validates the identity, and
Microsoft signs the package during Store submission. Complete the listing,
capability declarations, certification, and submission options in Partner
Center. `runFullTrust` is a restricted capability and may require Store
review. The unsigned `.msix` is not a GitHub direct-download installer.

**Packaged runtime behavior:** Store-installed packages register URL and HTML
handlers through the manifest's `uap:Protocol` and `uap:FileTypeAssociation`
extensions, rather than Linh's unpackaged HKCU `LinhURL` and
`RegisteredApplications` writes, and HKCU\Software\Classes is virtualized for
a packaged process regardless. `platform::windows::packaging::is_packaged()`
detects package identity with `GetCurrentPackageFullName`
(`ERROR_INSUFFICIENT_BUFFER` means packaged, `APPMODEL_ERROR_NO_PACKAGE` means
an unpackaged NSIS install); when packaged, `refresh_registration_if_stale()`
is a no-op and `set_default_browser()` only opens the Default Apps settings
page instead of writing HKCU. The manifest does **not** declare an explicit
ProgID for its `uap:Protocol` extensions (`desktop7:ProgId` needs Windows 10
Build 19645, above this package's `MinVersion` of 1809 — do not raise it just
for that), so a packaged install gets an opaque, Windows-generated ProgID
instead of `platform::windows::default_browser::PROG_ID`.
`is_default_browser()` therefore resolves a packaged scheme's `UserChoice`
ProgID under `HKCR\<ProgId>\Application`'s `AppUserModelId` value and compares
it against this process' own AUMID (`GetCurrentApplicationUserModelId()`)
instead of comparing ProgID names. Verify HTTP and HTTPS selection,
activation, update, and uninstall on an actual Store-installed package before
submission.

## Identifier change

Linh's app identifier changed from `dev.tkhieu.linh` to `dev.hieutk.linh` so
it matches a domain the owner controls, as Flathub requires. This only
renames the bundle identity; the GitHub repo (`tkhieu/linh`), the updater
endpoint, and the updater's signing pubkey are unaffected.

What is **not** affected, and needed no migration, because none of it was
ever keyed by the identifier in the first place:

- Linh's own config/state (`~/.config/linh`, `~/.local/state/linh`, and
  their macOS/Windows equivalents under `Linh`) — these are fixed, literal
  paths in each platform's `dirs.rs`, never Tauri's identifier-derived
  `app_config_dir`/`app_data_dir`.
- The Linux `.desktop` file name (`linh.desktop`) — fixed by
  `LINH_DESKTOP_ID`/`productName`, not the identifier, so an existing
  `xdg-settings default-web-browser linh.desktop` registration survives
  untouched.
- The Windows `LinhURL` ProgID, `Software\Linh\Capabilities` key, and
  `RegisteredApplications` entry — all literal, not identifier-derived.
- The MSIX identity (`MSIX_IDENTITY_NAME`/`MSIX_PUBLISHER`/
  `MSIX_PUBLISHER_DISPLAY_NAME`) — separate repository variables.
- The NSIS uninstall registry key — keyed by `productName` (`linh`), not
  the identifier. The NSIS *custom install location* lookup
  (`Software\<manufacturer>\<productName>`) would not have been: the
  manufacturer defaults to the identifier's second label
  (`tauri-bundler` 2.10.4's `nsis/mod.rs`), so it would have silently
  moved from `tkhieu` to `hieutk` and stopped finding anyone who installed
  to a non-default directory. `bundle.publisher` is now set explicitly to
  `tkhieu` in `tauri.conf.json` so the manufacturer — and this key — stay
  fixed across identifier changes; see below.

What **is** affected, and what an existing user must do after upgrading:

- **macOS:** LaunchServices keys the default `http`/`https` handler by
  bundle id, so an install that was default under the old id reports as
  *not* default after upgrading. Open Settings and choose **Make Linh
  your default browser** again; this is the same banner a fresh install
  shows, not a new code path.
- **Linux:** none. The desktop id and config paths are unaffected (see
  above), so the existing default-browser registration and config keep
  working with no action.
- **Windows:** none. The ProgID/registry integration is unaffected (see
  above). `bundle.publisher: "tkhieu"` keeps the installer's manufacturer
  — and the Programs & Features "Publisher" field — pinned to `tkhieu`
  instead of drifting to `hieutk`, and with it the custom-install-location
  lookup above. A Start Menu or Desktop shortcut created by an older
  install keeps working and points at the right exe either way; the NSIS
  `NSIS_HOOK_POSTINSTALL` hook in `minimum-version.nsh` re-applies the
  shortcut's AppUserModelID on every install/upgrade so existing shortcuts
  pick up the new `dev.hieutk.linh` id too — otherwise toast notifications
  for an upgraded install would not show or would be misattributed, since
  the installer's own shortcut code only sets the AUMID when creating a
  shortcut or migrating one from an old binary name.
- **Everywhere:** Linh is a tray-resident background process, and
  `tauri-plugin-single-instance`'s D-Bus name (Linux) / named mutex
  (Windows) / Unix domain socket under `/tmp` (macOS,
  `platform_impl/macos.rs::socket_path`) is derived from the identifier at
  build time. An old-id
  instance still running and a newly launched new-id instance will not
  recognize each other as "the same app" for the one upgrade that crosses
  this rename, so both could end up running at once if the old instance
  is never quit. Fully quit Linh (tray icon → **Quit**) once after
  updating to this release, then relaunch, to avoid that. This is a
  one-time condition for this release only; every later upgrade is
  unaffected since both the running and the newly installed copy share
  the same (new) identifier.
- Linh never reads or writes the identifier-keyed WebKit/WebView local
  storage and cache directory itself (no `localStorage`, `indexedDB`, or
  similar in `ui/src`), so the old one is simply orphaned — a few MB, not
  cleaned up automatically, same as any Tauri app identifier change. No
  action needed.

Self-recognition (desktop/Flatpak self-exclusion from Linh's own browser
list, so Linh is never offered as a target to launch itself) checks both
the current and legacy ids: `platform::linux::scan::is_self`,
`platform::macos::browsers::is_self`. The macOS default-browser check
(`platform::macos::default_browser::is_default_browser`) deliberately does
**not** do this — it must treat the legacy id as "needs re-selection", not
as "still Linh".

## Updater signing key

Linh's self-updater (`tauri-plugin-updater`) trusts one minisign key pair.
`src-tauri/tauri.conf.json`'s `plugins.updater.pubkey` carries the public
half; every installed copy of Linh verifies update signatures against it.

- **Where the key lives:** the owner generated it with `tauri signer generate`
  on their own machine, under `~/.tauri/linh-updater/`: the private key, its
  password, and the public key, each file mode `600`. The owner keeps an
  offline backup of the private key and password outside that machine.
- **The two secrets:** `TAURI_SIGNING_PRIVATE_KEY` (the private key contents)
  and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are repository secrets, never
  repository variables. `release.yml` passes them to `cargo tauri signer
  sign` as environment variables for the Fedora `.rpm`, Ubuntu `.deb`,
  Windows NSIS `.exe`, and the macOS `linh.app.tar.gz` (built from the `.app`
  bundle, packaged next to the `.dmg`). Each produces a `.sig` file uploaded
  alongside its artifact. The MSIX is never signed for the updater: Store
  installs update through the Store or an `.appinstaller` file, not this
  plugin. A `workflow_dispatch` dry run signs too, so CI proves signing
  works; it only skips, with a warning, when the two secrets are absent
  (e.g. a fork running the workflow manually). On a tag push, or a manual
  non-dry run, missing secrets fail the job.
- **Losing the key:** without the private key and password, no future
  release can produce a signature any existing install trusts, because each
  install's compiled-in `pubkey` only accepts signatures from the matching
  private key. Every installed copy would be permanently stuck on its
  current version, with no way to push a fix. This is why the offline
  backup exists.
- **Rotation is not instant — read this before touching the key.**
  `tauri-plugin-updater` (checked against 2.14.1's
  [configuration struct](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.14.1/plugins/updater/src/config.rs#L107),
  [download path](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.14.1/plugins/updater/src/updater.rs#L841-L848),
  and [signature verifier](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.14.1/plugins/updater/src/updater.rs#L901-L908):
  `config.rs:116` declares one `pubkey: String`, and `updater.rs:905` passes
  that key to the verifier) trusts exactly **one** public key, and it is the one **compiled
  into the already-installed binary**, not whatever `tauri.conf.json` says
  today. An install that never downloads a build containing the new pubkey
  has no way to verify anything signed only with the new private key — it
  is not "slower to update," it **cannot** update at all, which is the same
  failure mode as losing the key outright for that install.

  The only safe order:
  1. Generate the new key pair. Keep the old private key and password; do
     not delete them yet.
  2. Ship one release that changes only `plugins.updater.pubkey` to the new
     public key, still **signed with the old private key** (CI secrets
     stay on the old key for this release). Every install that applies
     this release now carries the new pubkey.
  3. Keep that release's `latest.json` as the download target — i.e. do
     not point `latest.json` at a *newer*, new-key-signed release — for a
     window generous enough that installs which only check every 24h (plus
     machines that were off for a while) have had the chance to pick it up.
     There is no enforced minimum here; treat a few weeks as a floor, and
     check install telemetry if any exists before shortening it.
  4. Only then switch CI's secrets to the new private key and resume normal
     releases.
  5. Any install that was offline through the whole window, or skipped
     straight from an old pre-rotation version to a new post-rotation one
     some other way, ends up in the same place as key loss for that one
     install: it cannot verify a new-key release and needs a manual
     reinstall. This is expected and unavoidable with a single-pubkey
     updater — it is not a bug to fix later.
- **Verifying a `.sig` locally.** Both the public key in `tauri.conf.json`
  (and in `~/.tauri/linh-updater/linh-updater.key.pub`) and every `.sig`
  file `cargo tauri signer sign` writes are themselves base64 text wrapping
  the real minisign file format, so `minisign -V` needs the *decoded* forms,
  not these files directly:
  ```sh
  base64 -d linh-updater.key.pub > linh-updater.key.pub.decoded
  base64 -d linh-0.3.2-1.x86_64.rpm.sig > linh-0.3.2-1.x86_64.rpm.sig.decoded
  minisign -V -p linh-updater.key.pub.decoded \
    -m linh-0.3.2-1.x86_64.rpm -x linh-0.3.2-1.x86_64.rpm.sig.decoded
  ```
  Run against a real dry-run artifact (`tkhieu/linh` run `45983975606`,
  public key only), this printed:
  ```
  Signature and comment signature verified
  Trusted comment: timestamp:1795776596	file:linh-0.3.2-1.x86_64.rpm	version:0.3.2
  ```
  and the same command against a deliberately corrupted copy of the file
  printed `Signature verification failed` and exited non-zero, confirming
  the check actually rejects a tampered artifact. `cargo tauri signer` has
  no `verify` subcommand, which is why this needs the standalone `minisign`
  CLI. At update time the plugin does the equivalent check itself: it
  downloads the artifact named in `latest.json`, base64-decodes the
  matching `signature` field, and verifies it against the pubkey compiled
  into that install before installing anything.

References: [Tauri macOS signing and notarization](https://v2.tauri.app/distribute/sign/macos/),
[Tauri environment variables](https://v2.tauri.app/reference/environment-variables/),
[Microsoft manual desktop packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool),
[MakePri](https://learn.microsoft.com/en-us/windows/uwp/app-resources/makepri-exe-command-options),
[Store package uploads](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/upload-app-packages),
[Store readiness](https://learn.microsoft.com/en-us/windows/apps/publish/store-readiness-checklist),
[Store signing](https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide),
[package identity detection](https://learn.microsoft.com/en-us/windows/msix/detect-package-identity),
[Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids).
