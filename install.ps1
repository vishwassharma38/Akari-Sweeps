<#
.SYNOPSIS
  Build + install Akari-Sweeps V00.1 (one binary, six command names).
.DESCRIPTION
  Strategy (documented choice): a single compiled Rust binary is COPIED under
  all six public names. Dispatch reads the executable file stem, so every
  alias is a full native executable: no CWD dependence, no BAT wrapper,
  arguments forwarded naturally, exit codes propagate natively.
  Install layout:
    <InstallDir>\akari.exe
    <InstallDir>\akariclean.exe   (copy)
    <InstallDir>\akarihealth.exe  (copy)
    <InstallDir>\akariall.exe     (copy)
    <InstallDir>\akariabout.exe   (copy)
    <InstallDir>\akarihelp.exe    (copy)
    <InstallDir>\assets\akari\{sweep,idle,health}\*.png
    <InstallDir>\assets\akari\about\frames\*.png
  Adds <InstallDir> to the USER PATH (undo via uninstall.ps1).
.PARAMETER InstallDir
  Defaults to "$env:LOCALAPPDATA\Akari-Sweeps".
.PARAMETER NoBuild
  Skip `cargo build --release` (reuse existing target\release\akari-sweeps.exe).
#>
param(
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Akari-Sweeps"),
  [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $NoBuild) {
  Write-Host "Building release binary..." -ForegroundColor Cyan
  cargo build --release --manifest-path (Join-Path $Root "Cargo.toml")
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$Src = Join-Path $Root "target\release\akari-sweeps.exe"
if (-not (Test-Path -LiteralPath $Src)) {
  Write-Error "Release binary not found: $Src (run without -NoBuild first)."
  exit 1
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$Names = @("akari", "akariclean", "akarihealth", "akariall", "akariabout", "akarihelp")
foreach ($n in $Names) {
  Copy-Item -LiteralPath $Src -Destination (Join-Path $InstallDir "$n.exe") -Force
  Write-Host "  installed $n.exe"
}

# Assets live next to the exes so every alias resolves them identically.
$AssetsSrc = Join-Path $Root "assets"
$AssetsDst = Join-Path $InstallDir "assets"
if (Test-Path -LiteralPath $AssetsSrc) {
  Copy-Item -Path $AssetsSrc -Destination $AssetsDst -Recurse -Force
  Write-Host "  installed assets/"
}

# User PATH (reversible; machine PATH untouched).
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
  [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
  Write-Host "Added to user PATH: $InstallDir" -ForegroundColor Green
  Write-Host "Restart your terminal to pick up the new PATH."
} else {
  Write-Host "User PATH already contains $InstallDir"
}

Write-Host ""
Write-Host "Done. Try: akarihelp   (new terminal)" -ForegroundColor Green
Write-Host "Undo:   .\uninstall.ps1 [-InstallDir '$InstallDir']"
