<#
Checks whether a built `linh` exe actually contains the self-updater
(`tauri-plugin-updater`), by searching its raw bytes for a string the
plugin's own `Error` enum formats with (`EmptyEndpoints`,
tauri-plugin-updater-2.14.1/src/error.rs:13) — never by running the exe
(see package-msix.ps1's own notes on why: linh.exe is a GUI-subsystem
binary with no reliable stdout non-interactively) or by trusting a
sidecar file (a stamp file next to the exe doesn't prove what the exe
itself actually contains; see docs/release.md). Confirmed empirically on
Linux: the string appears exactly once in a default (self-update on)
release build, and zero times in an `LINH_DIST=msstore
--no-default-features` build, in both cases via `thiserror`'s generated
`Display` impl for `tauri_plugin_updater::Error` — compiled in and
retained by the linker because it's reachable from `main` through
`updates::describe_error`, not because anything marks it specially.

Used two ways:
- `-Expect absent`: the negative check `package-msix.ps1` runs on the
  Store exe before packaging it — refuses to package anything the
  self-updater marker is still present in.
- `-Expect present`: the positive control `release.yml` runs on the
  NSIS/github exe — fails loudly if the marker is ever missing from a
  build that is supposed to have it (e.g. if a future tauri-plugin-updater
  upgrade renames or removes this string), instead of the negative check
  above silently passing for the wrong reason.
#>
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][ValidateSet('present', 'absent')][string]$Expect
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$marker = 'Updater does not have any endpoints set.'
$exeBytes = [System.IO.File]::ReadAllBytes($ExePath)
# Latin-1 maps every byte 0-255 to one char losslessly, so this never
# throws on non-UTF8 bytes elsewhere in the binary, while still finding
# the marker, which is plain ASCII.
$exeText = [System.Text.Encoding]::Latin1.GetString($exeBytes)
$found = $exeText.Contains($marker)

if ($Expect -eq 'present' -and -not $found) {
    throw "$ExePath does not contain the self-updater marker ('$marker') - expected it to, since this build is supposed to ship the self-updater. Either tauri-plugin-updater changed this string (update the marker here and in package-msix.ps1), or this build is missing the plugin when it shouldn't be."
}
if ($Expect -eq 'absent' -and $found) {
    throw "$ExePath contains the self-updater marker ('$marker') - refusing to continue. Store policy (Microsoft Store, Apple App Store guideline 2.5.2) forbids shipping the self-updater at all; this build must be LINH_DIST=msstore or mas with --no-default-features."
}

Write-Host "OK: $ExePath - self-updater marker is $Expect, as expected"
