# Releasing Linh

Linh's public GitHub releases use GPL-3.0-only. GitHub's Windows NSIS download is
unsigned; the Microsoft Store is the signed Windows channel. The Store MSIX
artifact is for Partner Center submission, not direct installation.

## Release steps

1. Set the same `MAJOR.MINOR.PATCH` version in `Cargo.toml` under
   `[workspace.package]`, `src-tauri/tauri.conf.json`, and `ui/package.json`.
   Run `python3 scripts/check-release-version.py`. The workflow rejects a tag
   unless all three versions equal the `vMAJOR.MINOR.PATCH` tag.
2. Run **Release** with `workflow_dispatch` and `dry_run=true` on a branch
   after the workflow is available on the default branch. Inspect all four
   build jobs and the complete `linh-release-assets` artifact, including
   `SHA256SUMS`. Dry runs create no GitHub Release. The initial PR
   can use a temporary pull request trigger for verification before merge.
3. Merge the change, then push the `vMAJOR.MINOR.PATCH` tag. The tag build
   creates a **draft** GitHub Release with `.rpm`, `.deb`, unsigned NSIS `.exe`,
   Store `.msix`, universal macOS `.dmg`, `SHA256SUMS`, and GitHub-generated
   notes from changes since the previous tag. Check the files and notes,
   then publish the draft manually.
4. Submit the `.msix` separately in Partner Center and manually test Windows
   default-browser selection and URL forwarding on the Store-installed app.
   Also test the macOS DMG on a Mac before publishing the GitHub draft.

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

**Runtime follow-up before Store publication:** Store-installed packages
register URL and HTML handlers through the manifest's `uap:Protocol` and
`uap:FileTypeAssociation` extensions, rather than Linh's current HKCU
`LinhURL` and `RegisteredApplications` writes. Linh's Windows
`set_default_browser()` and `refresh_registration_if_stale()` currently write
HKCU, and `is_default_browser()` checks only the `LinhURL` ProgID. A packaged
install may therefore make duplicate/stale registrations and report the wrong
default status. Detect package identity with `GetCurrentPackageFullName`
(`APPMODEL_ERROR_NO_PACKAGE` means an unpackaged NSIS install), skip HKCU
registration and repair in packaged mode, and read the packaged association
when checking default status. Verify HTTP and HTTPS selection, activation,
update, and uninstall on an actual Store-installed package. This release task
does not change Rust runtime behavior.

References: [Tauri macOS signing and notarization](https://v2.tauri.app/distribute/sign/macos/),
[Tauri environment variables](https://v2.tauri.app/reference/environment-variables/),
[Microsoft manual desktop packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool),
[MakePri](https://learn.microsoft.com/en-us/windows/uwp/app-resources/makepri-exe-command-options),
[Store package uploads](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/upload-app-packages),
[Store readiness](https://learn.microsoft.com/en-us/windows/apps/publish/store-readiness-checklist),
[Store signing](https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide),
[package identity detection](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getcurrentpackagefullname).
