# Store submissions for Linh

Status: APPROVED 2026-11-27. Owner decisions: order Microsoft Store → Copr + PPA → Flathub; the Mac App Store reduced build is WANTED (phase 8 in scope); Snap is DROPPED (not deferred); the app id becomes `dev.hieutk.linh` (rename PR first, before any store phase).
Created: 2026-11-27
Base: `main` at v0.4.0 (`2c2a169`), which has the in-app updater (#35-#37, #39)

## Outcome

Linh is installable from the stores where its core job (being the default browser, then opening each link in the right browser, profile or app) still works. Each store build turns off Linh's own updater, because the store delivers updates.

## Verdict per store

Evidence comes from two research passes and an independent cross-check by an agent from the other vendor, all on 2026-11-27. The sources are linked at the end.

| Store | Verdict | Why |
|---|---|---|
| **Microsoft Store (MSIX)** | **Go: first** | The MSIX build, manifest, `runFullTrust`, http/https `uap:Protocol`, the packaged-default check (#32) and the updater being off for MSIX (#37) already exist. Microsoft signs on submission, so no certificate is needed. An individual Partner Center account is free. What remains is mostly listing work. |
| **Flathub (Flatpak)** | **Go: after the shared store work** | Precedent: Junction and Switchyard are on Flathub holding `--talk-name=org.freedesktop.Flatpak` and launch host apps via `flatpak-spawn --host`. A Flatpak that exports `x-scheme-handler/http;https` can be chosen in GNOME Default Apps. Needed: run native `Exec` launches through `flatpak-spawn --host`, discovery via the read-only xdg-data applications dirs, the notification portal, the updater off, a manifest with offline cargo/pnpm sources, metainfo. Acceptance is per review and not guaranteed. |
| **Fedora "store" (GNOME Software)** | **Go via Copr + Flathub** | GNOME Software shows Fedora RPMs and Flathub. An official Fedora package needs every Rust crate and npm dependency packaged or unbundled, a large ongoing effort, so it is not pursued. A Copr repo with AppStream generation on, plus the metainfo, makes the rpm installable and listable. Flathub covers the "store" listing. |
| **Ubuntu "store" (App Center)** | **Go via a Launchpad PPA; Snap deferred** | App Center searches snaps and official Ubuntu debs; PPA packages are installable but not shown as searchable there (not established). A strict snap cannot launch host apps. Classic confinement needs a forum request and review with an uncertain outcome. The PPA reuses the existing deb build. |
| **Snap Store** | **Dropped by the owner** | Not pursued. Ubuntu users get the PPA or Flathub. |
| **Mac App Store** | **No-go as a full app; a reduced build is possible** | App Sandbox is mandatory. `NSWorkspace.open(…withApplicationAt:configuration:)` can open a URL in a chosen browser, but Apple documents that `OpenConfiguration.arguments` is **ignored when sandboxed**, so profile and private windows are impossible. Setting the default browser has a consent-based API (`setDefaultApplication`); whether it works when sandboxed is not proven. The `sips` icon extraction, the `open -n --args` launches and the updater must go. The recommendation is notarized direct distribution, which needs the Developer ID secrets already documented in `docs/release.md`. |

## Phases (one PR each, cross-reviewed)

| # | PR | Main files | Depends on |
|---|---|---|---|
| 1 | **Store build flavour.** One build-time switch, `LINH_DIST=github\|msstore\|flathub\|copr\|ppa\|mas` (a cargo feature or env var baked in at compile time), that sets the update capability to `None` for store channels. Settings → Updates shows "Updates come from <store>". Tests. | `src-tauri/src/updates.rs`, `build.rs`, `ui/src/settings/UpdatesTab.tsx` | none |
| 2 | **AppStream metainfo and desktop polish.** `dev.tkhieu.linh.metainfo.xml` (summary, description, ≥2 screenshots, OARS 1.1, releases, URLs), validated with `appstreamcli validate` in CI, installed by the rpm and deb. | `packaging/linux/*.metainfo.xml`, `tauri.conf.json`, CI | owner screenshots |
| 3 | **Microsoft Store listing.** The real MSIX identity variables, Store assets (logos and sizes), the `LINH_DIST=msstore` build in `release.yml`, and a submission checklist in `docs/release.md`. Manual upload in Partner Center to start with. | `packaging/windows/*`, `release.yml`, `docs/release.md` | 1; the owner's Partner Center account |
| 4 | **Copr.** An rpm spec or a Copr build from the release SRPM/rpm, with AppStream generation on, and a docs install line (`dnf copr enable tkhieu/linh`). | `packaging/copr/`, docs | 1, 2; owner Copr account |
| 5 | **Launchpad PPA.** A `debian/` source package (vendored cargo and pnpm deps, because Launchpad builds offline), `dput` instructions, docs. | `packaging/debian/`, docs | 1, 2; owner Launchpad account and GPG key |
| 6 | **Flatpak sandbox support.** Detect `/.flatpak-info`, launch native `Exec` via `flatpak-spawn --host`, launch Flatpak browsers via `flatpak-spawn --host flatpak run …`, the notification portal, and default-browser through the exported `MimeType` plus GNOME Default Apps (Make default opens Settings). Tests. | `src-tauri/src/platform/linux/*`, `crates/linh-core/src/launch.rs` | 1 |
| 7 | **Flathub manifest.** A `dev.tkhieu.linh.yml` with GNOME runtime webkit2gtk-4.1, `flatpak-builder-tools` cargo and node sources, the finish-args (`--talk-name=org.freedesktop.Flatpak` with justification, `--filesystem=xdg-data/applications:ro`, `/var/lib/flatpak/exports/share/applications:ro`, network, the notification portal), a local `flatpak-builder` CI build, then the PR to `flathub/flathub`. | `packaging/flatpak/*`, CI | 2, 6; owner GitHub for the Flathub PR |
| 8 | **Mac App Store reduced build (owner chose it).** `LINH_DIST=mas`: App Sandbox entitlements, `NSWorkspace.open(withApplicationAt:)` without arguments, profile/private hidden, no `sips`, consent-based default via `setDefaultApplication`, Apple Distribution signing. | `src-tauri/src/platform/macos/*`, entitlements, CI | 1; Apple Developer Program |

## Owner prerequisites

- **Microsoft:** a Partner Center individual account (free), the reserved app name "Linh", the publisher identity values for `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER` and `MSIX_PUBLISHER_DISPLAY_NAME`, a privacy policy URL and a support URL.
- **Flathub:** a GitHub account to open the submission PR, and agreement to the Flathub requirements (app id `dev.tkhieu.linh` needs control of `tkhieu.dev`, or a switch to `io.github.tkhieu.Linh`).
- **Copr:** a FAS account.
- **PPA:** a Launchpad account and a 4096-bit GPG key.
- **Mac:** an Apple Developer Program account ($99/year) for notarization, and also for the App Store if the reduced build is wanted.
- **All stores:** at least 2 real screenshots per platform, and a privacy policy page (Linh collects nothing, but stores require the page).

## Risks

- Flathub reviewers can still refuse `org.freedesktop.Flatpak`. The fallback is launching only Flatpak browsers, or the Copr route only.
- The app id: Flathub requires proof of domain control for `dev.tkhieu.*`. Changing the id also changes the config/state paths and the updater identity, which would be a migration.
- Store builds and GitHub builds must never cross-update. The `LINH_DIST` switch, set at compile time, prevents it.
- A Mac App Store reduced build may confuse users. Deferring it is acceptable.

## Open decisions (owner)

1. ~~Order~~: DECIDED: Microsoft Store → Copr + PPA → Flathub.
2. ~~Mac App Store~~: DECIDED: also ship the reduced MAS build (phase 8). Notarized direct download remains the full-featured macOS channel.
3. ~~Flathub app id~~: DECIDED 2026-11-27: the owner controls hieutk.dev, so every bundle/app id becomes `dev.hieutk.linh` (rename PR before the first published updater release).
4. ~~Snap~~: DECIDED: dropped.

## Sources (checked 2026-11-27)

- Junction Flathub manifest: https://github.com/flathub/re.sonny.Junction/blob/master/re.sonny.Junction.json
- Switchyard: https://github.com/flathub/io.github.alyraffauf.Switchyard
- GNOME Default Apps source: https://github.com/GNOME/gnome-control-center/blob/main/panels/default-apps/cc-default-apps-panel.c
- NSWorkspace arguments ignored when sandboxed: https://developer.apple.com/documentation/appkit/nsworkspace/openconfiguration/arguments
- NSWorkspace setDefaultApplication: https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopenurlswithscheme:completion:)
- MS Store distribution and accounts: https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/partner-center-developer-account
- Copr AppStream: https://docs.copr.fedorainfracloud.org/user_documentation.html
- Ubuntu 26.04 App Center: https://ubuntu.com/desktop/docs/en/26.04/explanation/snap-and-deb-packages/
- Snap confinement: https://snapcraft.io/docs/snap-confinement
- Flathub metainfo guidelines: https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines
