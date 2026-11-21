# Builds the Windows installer, dist\bluebubbles-gpui_<version>_x64.msi,
# on Windows. Needs what `cargo build` does there (see the README), plus
#   cargo install cargo-wix
# and the WiX Toolset 3 binaries (wix314-binaries.zip from
# github.com/wixtoolset/wix3/releases), unpacked anywhere: pass that
# folder as -WixBin, or have WiX's bin folder on the PATH.
#
#   scripts\package-msi.ps1 [-Toolchain 1.99.0] [-WixBin C:\tools\wix314]

param(
    [string]$Toolchain = "",
    [string]$WixBin = ""
)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$cargo = @()
if ($Toolchain) { $cargo += "+$Toolchain" }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$msi = "dist\bluebubbles-gpui_${version}_x64.msi"
New-Item -ItemType Directory -Force dist | Out-Null

cargo @cargo build --release --locked -p bluebubbles-gpui
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# cargo-wix adds WiX's Util extension (for closing the running app)
# itself, seeing its namespace in main.wxs.
$wix = @("wix", "-p", "bluebubbles-gpui", "--no-build", "--nocapture", "-o", $msi)
if ($WixBin) { $wix += @("-b", $WixBin) }
cargo @cargo @wix
if ($LASTEXITCODE) { exit $LASTEXITCODE }
Write-Output $msi
