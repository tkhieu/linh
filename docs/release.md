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
   Add one `<release>` entry for the new version to
   `packaging/linux/dev.hieutk.linh.metainfo.xml` when the release is
   published, using its publication date and a brief user-facing description.
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

## AppStream metainfo

The RPM and DEB install `packaging/linux/dev.hieutk.linh.metainfo.xml` under
`/usr/share/metainfo/`. Keep its release list in sync with published tags as
part of each release bump. Validate edits with
`appstreamcli validate --no-net --explain packaging/linux/dev.hieutk.linh.metainfo.xml`.
Real screenshots are still needed; see `docs/screenshots/README.md`.

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
the arguments"*.)

Every command below starts from the repository root — `--target-dir`
resolves against `cargo`'s own current directory, while the exe path,
`LINH_STORE_EXE_PATH`, and `package-msix.ps1` itself all assume the repo
root, so a `cd src-tauri` left in effect across the whole sequence (as an
earlier version of these docs implied) breaks the rest of it. `cd`s back
out immediately after the one `cargo tauri build` call that needs
`src-tauri`. The bash commands below (every dist except `mas`, whose `app`
bundle target only exists in macOS's own `tauri-cli`) were actually run
from this checkout, start to finish, to confirm both the `--` requirement
and this directory structure: for the `msstore` sequence specifically,
after returning to the repo root, `target/msstore/release/linh` (the
`.exe` on Windows) and its build stamp
`target/msstore/release/linh.dist-marker.txt` both resolved at exactly the
paths `LINH_STORE_EXE_PATH` and `package-msix.ps1` expect.

```sh
# GitHub (default): the in-app updater is this install's update path.
(cd src-tauri && cargo tauri build --bundles rpm)   # or deb/nsis/dmg, per platform

# Mac App Store: `self-update` must be off, run on macOS.
(cd src-tauri && LINH_DIST=mas cargo tauri build --bundles app -- --no-default-features)

# Flathub / Copr / PPA: `self-update` stays on (no store policy forbids
# it here), but it never runs — the distro/Flathub updates these installs,
# not Linh. Each reuses the existing rpm/deb bundle; the packaging work
# specific to each channel (the Copr spec, the PPA source package, the
# Flathub manifest) is tracked separately in the store-submissions plan
# (phases 4, 5, 7) and isn't built by this command on its own yet.
(cd src-tauri && LINH_DIST=copr cargo tauri build --bundles rpm)
(cd src-tauri && LINH_DIST=ppa cargo tauri build --bundles deb)
(cd src-tauri && LINH_DIST=flathub cargo tauri build --bundles rpm)
```

Microsoft Store (Windows, PowerShell — this is exactly release.yml's "Build
Windows Store binary" and "Package unsigned Windows Store MSIX" steps, as
one sequence): `self-update` must be off, and the exe that ends up in the
MSIX must not be the same one the NSIS installer just built, so it's built
into its own `--target-dir`. The build itself needs `src-tauri` as its
working directory; everything after it — the exe path, `package-msix.ps1`
— needs the repo root, hence the explicit `Set-Location`s:

```powershell
Set-Location src-tauri
$env:LINH_DIST = 'msstore'
cargo tauri build --no-bundle -- --no-default-features --target-dir ../target/msstore
Set-Location ..
$env:LINH_STORE_EXE_PATH = 'target/msstore/release/linh.exe'
$env:LINH_VERSION = '0.4.1'
./scripts/package-msix.ps1
```

The packaging step's primary check reads the **exe itself**, not a
sidecar: `scripts/assert-self-updater-presence.ps1` searches the exe's
raw bytes for a string `tauri-plugin-updater` only emits when it's
actually compiled in — `thiserror`'s generated `Display` text for
`Error::EmptyEndpoints`, *"Updater does not have any endpoints set."*
(`tauri-plugin-updater-2.14.1/src/error.rs:13`). That impl is reachable
from `main` through `updates::describe_error`, so the string is part of
genuinely live code, not an inert marker a linker is free to discard —
confirmed empirically on Linux: it appears exactly once in a default
(`github`, self-update on) release build and zero times in an
`LINH_DIST=msstore --no-default-features` build. `package-msix.ps1`
refuses to package any exe the string is present in.

Getting to a check that reads the exe itself took three attempts: running
`linh.exe --print-dist` and capturing its output crashed in real CI,
because `linh` is a GUI-subsystem binary (`windows_subsystem =
"windows"`) with no reliable stdout when spawned non-interactively
("failed printing to stdout: The pipe is being closed"); embedding a
purpose-built marker in the exe's data section via a `#[used] static`
(also checked by a raw byte search) also failed in real CI, because the
MSVC linker's default `/OPT:REF` silently stripped the *unreferenced*
static from the real Windows build, even though the identical code
survived the Linux build this was first verified against. Searching for
a string the plugin's own, already-reachable code emits sidesteps both:
nothing has to run, and nothing needed inventing had to survive the
linker on its own.

`package-msix.ps1` runs the byte search *first* (`assert-self-updater-presence.ps1
-Expect absent`), before it ever reads the stamp. `build.rs` only forbids
`self-update` being *on* for `msstore`/`mas` (`build.rs:28`) — it doesn't
require `self-update` to be *on* for `github`/`flathub`/`copr`/`ppa`, so
any of those four can also be built with `--no-default-features` and
would pass the byte search exactly as validly as a genuine `msstore` exe
does. The byte search alone can only rule out a build that still has the
self-updater (`github`/`flathub`/`copr`/`ppa` built the normal way, with
`self-update` left on); it can't tell `msstore` apart from `mas`, or from
one of those four built without the self-updater. The stamp check below
is what then rejects anything whose `LINH_DIST` isn't exactly `msstore`.

A sidecar stamp file, `linh.dist-marker.txt`, written by `build.rs`'s
`write_dist_stamp` right next to the compiled binary (e.g.
`LINH_BUILD_MARKER:dist=msstore;self_update=false`), is kept as this
**second**, narrower layer — not because it's load-bearing for the
self-updater question (the byte search above already answers that for
the common case), but because it's the only thing that confirms the exe
is the *specific* `msstore` channel, rather than `mas` or a
`github`/`flathub`/`copr`/`ppa` build that happened to also be built
without the self-updater. Checking only the exe and dropping the stamp
entirely was considered and rejected for exactly that gap.
Because a stamp is written before the exe it sits next to is linked, a
stale stamp next to a *later-replaced* exe would otherwise still read as
a confirmed build — `release.yml`'s cleanup step deletes `target/msstore`
before every Windows job build (so the exe and its stamp can only ever
come from the same build), and `package-msix.ps1` separately refuses to
package an exe whose `LastWriteTime` is older than its stamp's.

`release.yml` also runs the byte search the other way, as a positive
control, against the NSIS build's own `target/release/linh.exe`
(`LINH_DIST` unset, `self-update` on): it must contain the marker. This
is what actually protects the check's own future: if a later
`tauri-plugin-updater` upgrade ever renames or rewords that error
message, the positive control fails loudly right there, instead of the
Store packaging step's negative check silently "passing" for the wrong
reason (the string being absent because it no longer exists anywhere,
not because this exe lacks the plugin).

**Residual risk**: the check is a string present in the compiled output
of a specific crate version, not a structural guarantee — a future
`tauri-plugin-updater` release could still coincidentally reintroduce an
identical string for an unrelated reason (vanishingly unlikely for a
sentence this specific), or change the string in a way the positive
control catches but requires a human to then update both this marker and
the one in `assert-self-updater-presence.ps1`. There's no feasible way to
make "the plugin's code is absent" fully self-verifying without Apple's
or Microsoft's own store review, which this repeats a version of every
time a release is prepared.

CI's `build` job runs `cargo check` and `cargo clippy` a second time with
`LINH_DIST=msstore` and `--no-default-features` on every push, so the store
configuration can't silently rot even outside a release build. Separately,
`release.yml`'s Windows job already builds and packages the unsigned Store
MSIX on every run — including a `dry_run`, where it's just a downloadable
workflow artifact — and includes it in the `linh-windows`/release asset
set; nothing here submits it anywhere. Getting it in front of Microsoft
still needs a human: see "Store MSIX" below for the manual Partner Center
upload.

## Store MSIX

Tauri builds the unsigned NSIS installer using its existing Windows build
command, with the default `github` dist and the self-updater included. The
MSIX is a *different* binary: the release job separately builds
`target/msstore/release/linh.exe` with `LINH_DIST=msstore` and
`--no-default-features` (see "Store build flavour" above), never the NSIS
build's own `target/release/linh.exe`. The release job stages that msstore
exe, GPL license, MSIX manifest, and logo assets, then uses the Windows SDK
`MakePri.exe` and `MakeAppx.exe` to produce an unsigned x64 `.msix`. This
keeps the packaging step small and uses Microsoft's own package validator.
The manifest targets
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

## Mac App Store

The Mac App Store build is a reduced, sandboxed version of Linh. The direct
GitHub DMG remains the full macOS build. Before submission, enroll in the
[Apple Developer Program](https://developer.apple.com/programs/enroll/)
($99/year), register `dev.hieutk.linh` as an App ID and create the matching
App Store Connect app record. Obtain an Apple Distribution (formerly
“3rd Party Mac Developer Application”) certificate, a Mac Installer
Distribution (formerly “3rd Party Mac Developer Installer”) certificate, and
a “Mac App Store Connect” provisioning profile for this App ID. Export both
certificates as base64-encoded `.p12` files, using the same export password.
Set these four GitHub Actions secrets (never commit their values):

| Secret | Value |
| --- | --- |
| `APPLE_MAS_CERTIFICATE` | Base64 application-signing `.p12` |
| `APPLE_MAS_CERTIFICATE_PASSWORD` | Export password for both `.p12` files |
| `APPLE_MAS_INSTALLER_CERTIFICATE` | Base64 installer-signing `.p12` |
| `APPLE_MAS_PROVISIONING_PROFILE` | Base64 Mac App Store Connect `.provisionprofile` |

On `macos-15`, the Release workflow builds the unsigned app with the exact
command below. [Tauri `--config`](https://v2.tauri.app/develop/configuration-files/#extending-the-configuration) merges JSON with the regular Tauri config;
`--no-default-features` and `--target-dir` pass through to Cargo after `--`.
The bundle ID stays `dev.hieutk.linh`, the category maps to
`public.app-category.utilities`, and the deployment target stays macOS 13.0.

```sh
cd src-tauri
LINH_DIST=mas cargo tauri build --bundles app --config tauri.mas.conf.json -- --no-default-features --target-dir ../target/mas
cd ..
```

`bundle.macOS.infoPlist` points to `src-tauri/macos/Info.mas.plist`, which
Tauri merges into the generated bundle `Info.plist`. Tauri also automatically
merges a root `src-tauri/Info.plist`, but using that name would affect the
GitHub build too. The MAS file sets `ITSAppUsesNonExemptEncryption=false`:
Linh uses the system's ordinary HTTPS facilities and contains no custom
nonexempt encryption. Confirm the answer against the final app before
submission.

### Entitlements

- `com.apple.security.app-sandbox=true` is required for Mac App Store apps.
  The overlay points Tauri to `macos/Linh.mas.entitlements`; entitlements
  become effective when code signing applies them. The workflow ad hoc signs
  a disposable copy and prints `codesign -d --entitlements :-` as proof.
- `com.apple.application-identifier` and
  `com.apple.developer.team-identifier` are added only for Store signing,
  using the Team ID from the provisioning profile. They are required to
  match Apple's App ID and cannot be filled before the owner has a profile.
- `com.apple.security.network.client` is omitted. Linh routes an incoming
  URL by asking macOS to open it in an installed browser; that browser makes
  the network request. The production WKWebView loads Tauri's bundled UI
  through a [local custom protocol](https://github.com/tauri-apps/tauri/blob/tauri-v2.12.5/crates/tauri/src/webview/mod.rs), and the MAS updater is compiled out.
  Neither action opens an outgoing connection from Linh. No temporary
  exception entitlements are requested.

The Release workflow checks the built binary for absence of
`Updater does not have any endpoints set.`, verifies Info.plist values,
archives the untouched unsigned `.app` as the `mas-app-unsigned` **workflow
artifact**, and never includes it in GitHub Release assets. When all four
MAS secrets exist, it imports both certificates into a temporary keychain,
embeds the profile at `Contents/embedded.provisionprofile`, signs the app
with its Apple Distribution identity and generated entitlements, and runs:

```sh
productbuild --component target/mas/release/bundle/macos/linh.app /Applications --sign "$installer_identity" --keychain "$keychain" Linh-mas.pkg
```

The signed `mas-pkg-signed` workflow artifact is for **manual** upload with
Apple Transporter; no workflow submits it to App Store Connect. Without the
four secrets, the signing step is skipped and the unsigned artifact still
builds. Test the signed package on a Mac before submission.

App Review notes: “Linh is a link router. Set it as the default browser,
then open an HTTP or HTTPS link. Linh selects an installed browser according
to the user's rules, or prompts the user, and asks macOS to open the link in
that browser. The Mac App Store build is sandboxed: browser profiles and
private-window launch options are unavailable, icon extraction via `sips`
is disabled, and it has no self-updater. Default-browser selection uses the
macOS consent prompt.”

For App Privacy, answer **Data Not Collected**: routing settings remain on
the device and Linh does not send link contents to a service. App Store
Connect also requires a privacy-policy URL; publish that page before
submission. Apple documents that
[`OpenConfiguration.arguments` is ignored for sandboxed apps](https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/arguments),
which is why MAS cannot offer profile and private-window launch arguments.
