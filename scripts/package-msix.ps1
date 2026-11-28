$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$version = $env:LINH_VERSION
if (-not $version -or $version -notmatch '^\d+\.\d+\.\d+$') {
    throw 'LINH_VERSION must be a three-part numeric version'
}
# No default/guessed path: the caller (release.yml) must point this at the
# binary it built specifically for the Store, with LINH_DIST=msstore and
# the self-update feature off — never the default github-flavour exe that
# also ships the NSIS installer, which still has the self-updater.
$sourceExe = $env:LINH_STORE_EXE_PATH
if (-not $sourceExe) {
    throw 'LINH_STORE_EXE_PATH must point at the msstore-flavour linh.exe (built with LINH_DIST=msstore and --no-default-features)'
}
$sourceExe = (Resolve-Path $sourceExe).Path
if (-not (Test-Path $sourceExe)) { throw "LINH_STORE_EXE_PATH does not exist: $sourceExe" }

# Assert the binary is actually what it claims to be before packaging it —
# run it and read back its own compiled-in `LINH_DIST`/`self-update`
# state (see `main.rs`'s `--print-dist` and `updates::print_dist_line`)
# rather than trusting that whatever built $sourceExe got the build
# command right. Store policy (Microsoft Store, Apple App Store guideline
# 2.5.2) forbids shipping the self-updater at all, so this fails the
# packaging step loudly instead of silently shipping it.
$distInfo = (& $sourceExe --print-dist) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "running '$sourceExe --print-dist' failed (exit $LASTEXITCODE): $distInfo" }
if ($distInfo -notmatch '^dist=msstore self_update=false$') {
    throw "LINH_STORE_EXE_PATH ($sourceExe) is not an msstore/self-update-off build: '$distInfo'"
}

$stage = Join-Path $root 'target/release/msix-stage'
$output = Join-Path $root "target/release/msix/Linh_$($version)_x64.msix"
$assets = Join-Path $stage 'Assets'
$config = Join-Path $root 'target/release/msix-priconfig.xml'
$manifest = Join-Path $stage 'AppxManifest.xml'
$sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$makeAppx = Get-ChildItem -Path $sdk -Filter makeappx.exe -Recurse |
    Where-Object { $_.DirectoryName -match '[\\/]x64$' } |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $makeAppx) { throw 'Windows SDK MakeAppx.exe was not found' }
$makePri = Join-Path $makeAppx.DirectoryName 'makepri.exe'
if (-not (Test-Path $makePri)) { throw 'Windows SDK MakePri.exe was not found' }

Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item $assets -ItemType Directory -Force | Out-Null
New-Item (Split-Path $output) -ItemType Directory -Force | Out-Null
Copy-Item $sourceExe (Join-Path $stage 'linh.exe')
Copy-Item (Join-Path $root 'LICENSE') $stage
Copy-Item (Join-Path $root 'packaging/windows/assets/*.png') $assets

$identityName = if ($env:MSIX_IDENTITY_NAME) { $env:MSIX_IDENTITY_NAME } else { 'Placeholder.Linh' }
$publisher = if ($env:MSIX_PUBLISHER) { $env:MSIX_PUBLISHER } else { 'CN=Placeholder' }
$displayName = if ($env:MSIX_PUBLISHER_DISPLAY_NAME) { $env:MSIX_PUBLISHER_DISPLAY_NAME } else { 'Placeholder Publisher' }
if ($identityName -eq 'Placeholder.Linh' -or $publisher -eq 'CN=Placeholder' -or $displayName -eq 'Placeholder Publisher') {
    Write-Warning 'MSIX identity uses placeholders. Set all three MSIX_* repository variables before Partner Center submission.'
}
$xml = Get-Content (Join-Path $root 'packaging/windows/AppxManifest.xml.in') -Raw
$xml = $xml.Replace('@IDENTITY_NAME@', [System.Security.SecurityElement]::Escape($identityName))
$xml = $xml.Replace('@PUBLISHER@', [System.Security.SecurityElement]::Escape($publisher))
$xml = $xml.Replace('@PUBLISHER_DISPLAY_NAME@', [System.Security.SecurityElement]::Escape($displayName))
$xml = $xml.Replace('@VERSION@', "$version.0")
[xml]$parsed = $xml
[System.IO.File]::WriteAllText($manifest, $xml, [System.Text.UTF8Encoding]::new($false))

& $makePri createconfig /cf $config /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw 'MakePri createconfig failed' }
& $makePri new /pr $stage /cf $config /mn $manifest /of (Join-Path $stage 'resources.pri') /o
if ($LASTEXITCODE -ne 0) { throw 'MakePri new failed' }
& $makeAppx pack /d $stage /p $output /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx pack failed' }
Write-Host "Unsigned Store submission package: $output"
