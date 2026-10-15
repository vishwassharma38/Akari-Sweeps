<#
.SYNOPSIS
  Undo install.ps1: remove the install dir and its user-PATH entry.
#>
param(
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Akari-Sweeps")
)

$ErrorActionPreference = "Stop"
$Names = @("akari", "akariclean", "akarihealth", "akariall", "akariabout", "akarihelp")
foreach ($n in $Names) {
  $p = Join-Path $InstallDir "$n.exe"
  if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force; Write-Host "  removed $n.exe" }
}
if (Test-Path -LiteralPath (Join-Path $InstallDir "assets")) {
  Remove-Item -LiteralPath (Join-Path $InstallDir "assets") -Recurse -Force
  Write-Host "  removed assets/"
}
if ((Test-Path -LiteralPath $InstallDir) -and @(Get-ChildItem -LiteralPath $InstallDir -Force).Count -eq 0) {
  Remove-Item -LiteralPath $InstallDir -Force
  Write-Host "  removed $InstallDir"
}

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$Parts = $UserPath -split ";" | Where-Object { $_ -ne "" -and $_ -ne $InstallDir }
[Environment]::SetEnvironmentVariable("Path", ($Parts -join ";"), "User")
Write-Host "Removed $InstallDir from user PATH (restart terminal)." -ForegroundColor Green
