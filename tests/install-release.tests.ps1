<#
.SYNOPSIS
  Deterministic tests for install-release.ps1.

.DESCRIPTION
  Runs on Windows PowerShell 5.1 without Pester, Rust, Cargo, or network
  access. The installer is dot-sourced so its functions can be exercised
  against isolated temporary directories, local ZIP packages, and a fake
  per-user PATH. The real user's PATH and any real Akari-Sweeps installation
  are never touched.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\install-release.tests.ps1
#>

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$script:InstallerPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'install-release.ps1'
if (-not (Test-Path -LiteralPath $script:InstallerPath -PathType Leaf)) {
    throw "Installer not found: $script:InstallerPath"
}
. $script:InstallerPath

$script:TestCount = 0
$script:FailureCount = 0
$script:FakeUserPath = ''
$script:MockDownloadDir = $null
$script:MockTag = 'v9.9.9'
$script:MockDownloadedUris = @()

# Test doubles: these replace the installer's environment and network edges.
function Get-InstallerUserPath {
    return $script:FakeUserPath
}

function Set-InstallerUserPath {
    param([string]$PathValue)
    $script:FakeUserPath = $PathValue
}

function Get-LatestReleaseTag {
    return $script:MockTag
}

function Get-RemoteFile {
    param(
        [string]$Uri,
        [string]$Destination
    )
    $script:MockDownloadedUris += $Uri
    if ([string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
        throw "Mock download is not configured for '$Uri'."
    }
    $source = Join-Path $script:MockDownloadDir ([IO.Path]::GetFileName($Destination))
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Mock release asset is missing: $Uri"
    }
    Copy-Item -LiteralPath $source -Destination $Destination -Force
}

$script:FailMoveDestination = $null
function Move-InstallerItem {
    param(
        [string]$Source,
        [string]$Destination
    )
    if (-not [string]::IsNullOrWhiteSpace($script:FailMoveDestination) -and
        [string]::Equals($Destination, $script:FailMoveDestination, [StringComparison]::OrdinalIgnoreCase) -and
        $Source.Contains('.akari-install-stage-')) {
        throw 'Injected move failure for transaction rollback test.'
    }
    Move-Item -LiteralPath $Source -Destination $Destination -Force
}

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Equal {
    param(
        $Actual,
        $Expected,
        [string]$Message
    )
    if ($Actual -ne $Expected) {
        throw "$Message (expected '$Expected', got '$Actual')"
    }
}

function Assert-Throws {
    param(
        [scriptblock]$Block,
        [string]$Message
    )
    $caught = $null
    try {
        & $Block | Out-Null
    } catch {
        $caught = $_
    }
    if ($null -eq $caught) {
        throw "Expected an error: $Message"
    }
    return $caught
}

function New-TestRoot {
    param([string]$Label)
    $root = Join-Path ([IO.Path]::GetTempPath()) ('akari-installer-test-' + $Label + '-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    return $root
}

function New-TestPackage {
    param(
        [string]$Root,
        [switch]$NoExecutable,
        [switch]$NoAssets,
        [switch]$MissingSpriteSet,
        [switch]$CorruptArchive
    )
    $stage = Join-Path $Root 'package-staging'
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    if (-not $NoExecutable) {
        [IO.File]::WriteAllBytes(
            (Join-Path $stage 'akari-sweeps.exe'),
            [byte[]](0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00)
        )
    }
    if (-not $NoAssets) {
        $sets = @('sweep', 'idle', 'health', 'about', 'help')
        if ($MissingSpriteSet) {
            $sets = @('sweep', 'idle', 'health', 'about')
        }
        foreach ($set in $sets) {
            $dir = Join-Path $stage ('assets\akari\' + $set)
            New-Item -ItemType Directory -Path $dir -Force | Out-Null
            [IO.File]::WriteAllBytes((Join-Path $dir 'frame_001.png'), [byte[]](1, 2, 3, 4))
        }
    }

    $packageName = 'akari-sweeps-windows-x86_64.zip'
    $zipPath = Join-Path $Root $packageName
    if ($CorruptArchive) {
        [IO.File]::WriteAllText($zipPath, 'this is not a zip archive')
    } else {
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -Force
    }
    $hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumPath = Join-Path $Root ($packageName + '.sha256')
    [IO.File]::WriteAllText(
        $checksumPath,
        ($hash + '  ' + $packageName + [Environment]::NewLine),
        (New-Object System.Text.UTF8Encoding($false))
    )
    return [pscustomobject]@{
        Root = $Root
        Package = $zipPath
        Checksum = $checksumPath
        PackageName = $packageName
        ChecksumName = $packageName + '.sha256'
        Stage = $stage
    }
}

function Set-MockDownload {
    param([string]$PackagePath, [string]$ChecksumPath)
    $directory = Join-Path ([IO.Path]::GetTempPath()) ('akari-installer-download-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    if (-not [string]::IsNullOrWhiteSpace($PackagePath) -and (Test-Path -LiteralPath $PackagePath)) {
        Copy-Item -LiteralPath $PackagePath -Destination (Join-Path $directory 'akari-sweeps-windows-x86_64.zip') -Force
    }
    if (-not [string]::IsNullOrWhiteSpace($ChecksumPath) -and (Test-Path -LiteralPath $ChecksumPath)) {
        Copy-Item -LiteralPath $ChecksumPath -Destination (Join-Path $directory 'akari-sweeps-windows-x86_64.zip.sha256') -Force
    }
    $script:MockDownloadDir = $directory
    return $directory
}

function Reset-TestState {
    param([string]$StartingPath = 'C:\Windows;C:\Tools\Akari')
    $script:FakeUserPath = $StartingPath
    $script:MockDownloadDir = $null
    $script:MockTag = 'v9.9.9'
    $script:MockDownloadedUris = @()
}

function New-TestZipArchive {
    param(
        [string]$Path,
        [object[]]$Entries
    )
    Add-Type -AssemblyName System.IO.Compression
    $file = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $archive = $null
    try {
        $archive = [IO.Compression.ZipArchive]::new(
            $file,
            [IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        foreach ($item in $Entries) {
            $entry = $archive.CreateEntry([string]$item.Name)
            $stream = $entry.Open()
            try {
                $bytes = [Text.Encoding]::UTF8.GetBytes([string]$item.Content)
                if ($bytes.Length -gt 0) {
                    $stream.Write($bytes, 0, $bytes.Length)
                }
            } finally {
                $stream.Dispose()
            }
        }
    } finally {
        if ($null -ne $archive) {
            $archive.Dispose()
        }
        $file.Dispose()
    }
}

function Test-Case {
    param(
        [string]$Name,
        [scriptblock]$Body
    )
    $script:TestCount++
    try {
        & $Body
        Write-Host ("PASS  " + $Name) -ForegroundColor Green
    } catch {
        $script:FailureCount++
        Write-Host ("FAIL  " + $Name) -ForegroundColor Red
        Write-Host ("      " + $_.Exception.Message) -ForegroundColor Red
    }
}

function Get-AkariTempDirs {
    return @(Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'akari-sweeps-install-*' -or $_.Name -like 'akari-sweeps-extract-*' } |
        Select-Object -ExpandProperty FullName)
}

Reset-TestState

Test-Case 'valid package installs six aliases, assets, and PATH entry' {
    $root = New-TestRoot 'valid'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install dir'
        Install-AkariSweeps -InstallDirectory $install

        foreach ($name in @('akari.exe', 'akariclean.exe', 'akarihealth.exe', 'akariall.exe', 'akariabout.exe', 'akarihelp.exe')) {
            Assert-True (Test-Path -LiteralPath (Join-Path $install $name) -PathType Leaf) "$name was not installed"
        }
        foreach ($set in @('sweep', 'idle', 'health', 'about', 'help')) {
            Assert-True (Test-Path -LiteralPath (Join-Path $install ('assets\akari\' + $set + '\frame_001.png')) -PathType Leaf) "$set asset missing"
        }
        $stored = (Get-FileHash -LiteralPath (Join-Path $install 'akari.exe') -Algorithm SHA256).Hash
        $expected = (Get-FileHash -LiteralPath (Join-Path $package.Stage 'akari-sweeps.exe') -Algorithm SHA256).Hash
        Assert-Equal $stored $expected 'installed executable differs from the release package'
        $parts = @($script:FakeUserPath -split ';')
        Assert-True ($parts -contains $install) 'install directory was not added to PATH'
        Assert-True ($parts -contains 'C:\Windows') 'unrelated PATH entry C:\Windows was lost'
        Assert-True ($parts -contains 'C:\Tools\Akari') 'unrelated PATH entry C:\Tools\Akari was lost'
        Assert-True ($script:MockDownloadedUris.Count -eq 2) 'expected exactly two release downloads'
        Assert-True ($script:MockDownloadedUris[0] -eq 'https://github.com/vishwassharma38/Akari-Sweeps/releases/download/v9.9.9/akari-sweeps-windows-x86_64.zip') 'wrong package URL'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'explicit version selects the matching release tag' {
    $root = New-TestRoot 'explicit-version'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') -RequestedVersion '1.2.3'
        Assert-True ($script:MockDownloadedUris[0].Contains('/releases/download/v1.2.3/')) 'explicit version did not build the matching tag URL'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'invalid checksum is rejected before installation' {
    $root = New-TestRoot 'bad-checksum'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        [IO.File]::WriteAllText($package.Checksum, (('0' * 64) + '  akari-sweeps-windows-x86_64.zip'))
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        Assert-Throws { Install-AkariSweeps -InstallDirectory $install } 'checksum mismatch must fail' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'akari.exe'))) 'files were installed after a checksum failure'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'missing release package fails cleanly' {
    $root = New-TestRoot 'missing-package'
    try {
        Reset-TestState
        $before = @(Get-AkariTempDirs)
        Set-MockDownload -PackagePath $null -ChecksumPath $null | Out-Null
        Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') } 'missing package must fail' | Out-Null
        $after = @(Get-AkariTempDirs)
        Assert-Equal $after.Count $before.Count 'temporary download directory was not cleaned up'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'failed download is reported and cleaned up' {
    $root = New-TestRoot 'download-failure'
    try {
        Reset-TestState
        $before = @(Get-AkariTempDirs)
        $script:MockDownloadDir = (Join-Path $root 'empty-mock')
        New-Item -ItemType Directory -Path $script:MockDownloadDir -Force | Out-Null
        Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') } 'failed download must fail' | Out-Null
        $after = @(Get-AkariTempDirs)
        Assert-Equal $after.Count $before.Count 'temporary download directory was not cleaned up after a download failure'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case 'missing executable in the package is rejected' {
    $root = New-TestRoot 'missing-exe'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root -NoExecutable
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        Assert-Throws { Install-AkariSweeps -InstallDirectory $install } 'missing executable must fail' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'akari.exe'))) 'files were installed from an incomplete package'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'missing assets directory is rejected' {
    $root = New-TestRoot 'missing-assets'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root -NoAssets
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') } 'missing assets must fail' | Out-Null
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'missing sprite set is rejected' {
    $root = New-TestRoot 'missing-sprite-set'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root -MissingSpriteSet
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') } 'missing sprite set must fail' | Out-Null
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'extraction failure is reported and cleaned up' {
    $root = New-TestRoot 'corrupt-archive'
    try {
        Reset-TestState
        $before = @(Get-AkariTempDirs)
        $package = New-TestPackage -Root $root -CorruptArchive
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path $root 'install') } 'corrupt archive must fail' | Out-Null
        $after = @(Get-AkariTempDirs)
        Assert-Equal $after.Count $before.Count 'temporary extraction directory was not cleaned up'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'existing installation directory preserves unrelated files' {
    $root = New-TestRoot 'existing-install'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $install -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $install 'settings.json'), '{"theme":"akari"}')
        [IO.File]::WriteAllText((Join-Path $install 'notes.txt'), 'user notes')
        Install-AkariSweeps -InstallDirectory $install
        Assert-Equal (Get-Content -LiteralPath (Join-Path $install 'settings.json') -Raw) '{"theme":"akari"}' 'user settings were overwritten'
        Assert-True (Test-Path -LiteralPath (Join-Path $install 'notes.txt') -PathType Leaf) 'unrelated user file was removed'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'repeated installation is idempotent and avoids duplicate PATH entries' {
    $root = New-TestRoot 'repeat'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        Install-AkariSweeps -InstallDirectory $install
        Install-AkariSweeps -InstallDirectory $install
        $occurrences = @($script:FakeUserPath -split ';' | Where-Object { $_ -eq $install }).Count
        Assert-Equal $occurrences 1 'repeated installation duplicated the PATH entry'
        Assert-True (Test-Path -LiteralPath (Join-Path $install 'akari.exe') -PathType Leaf) 'alias missing after repeated installation'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'PATH matching is case-insensitive and tolerates a trailing separator' {
    $root = New-TestRoot 'path-duplicate'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        $script:FakeUserPath = 'C:\Windows;' + $install.ToUpperInvariant() + '\;C:\Tools'
        Install-AkariSweeps -InstallDirectory $install
        $occurrences = @($script:FakeUserPath -split ';' | Where-Object {
            $_.Trim().TrimEnd([char[]]@('\', '/')).ToLowerInvariant() -eq $install.ToLowerInvariant()
        }).Count
        Assert-Equal $occurrences 1 'case/trailing-separator duplicate PATH entry was added'
        Assert-True (($script:FakeUserPath -split ';') -contains 'C:\Tools') 'unrelated PATH entry was lost'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'unsafe install directories are rejected' {
    $root = New-TestRoot 'unsafe-dir'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $driveRoot = [IO.Path]::GetPathRoot([IO.Path]::GetTempPath())
        Assert-Throws { Install-AkariSweeps -InstallDirectory $driveRoot } 'drive root install must be rejected' | Out-Null
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-Case 'invalid explicit version is rejected before downloading' {
    Reset-TestState
    Assert-Throws { Install-AkariSweeps -InstallDirectory (Join-Path ([IO.Path]::GetTempPath()) 'unused') -RequestedVersion 'not-a-version' } 'invalid version must fail' | Out-Null
    Assert-Equal $script:MockDownloadedUris.Count 0 'invalid version must not trigger a download'
}

Test-Case 'installer does not invoke Cargo or Rust' {
    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $script:InstallerPath,
        [ref]$tokens,
        [ref]$errors
    )
    Assert-Equal $errors.Count 0 'installer has PowerShell parse errors'
    $commands = $ast.FindAll(
        { param($node) $node -is [System.Management.Automation.Language.CommandAst] },
        $true
    )
    foreach ($command in $commands) {
        $name = $command.GetCommandName()
        if ($null -ne $name) {
            Assert-True ($name -notin @('cargo', 'rustc', 'rustup')) "installer unexpectedly invokes $name"
        }
    }
}

Test-Case 'release asset URLs are pinned to the expected repository, tag, and asset' {
    $valid = 'https://github.com/vishwassharma38/Akari-Sweeps/releases/download/v1.2.3/akari-sweeps-windows-x86_64.zip'
    [void](Assert-ReleaseAssetUrl -Url $valid -AssetName 'akari-sweeps-windows-x86_64.zip' -Tag 'v1.2.3')
    Assert-Throws {
        Assert-ReleaseAssetUrl -Url 'https://github.com/attacker/Akari-Sweeps/releases/download/v1.2.3/akari-sweeps-windows-x86_64.zip' -AssetName 'akari-sweeps-windows-x86_64.zip' -Tag 'v1.2.3'
    } 'foreign repository URL must be rejected' | Out-Null
    Assert-Throws {
        Assert-ReleaseAssetUrl -Url 'https://github.com/vishwassharma38/Akari-Sweeps/releases/download/v9.9.9/akari-sweeps-windows-x86_64.zip' -AssetName 'akari-sweeps-windows-x86_64.zip' -Tag 'v1.2.3'
    } 'mismatched release tag must be rejected' | Out-Null
    Assert-Throws {
        Assert-ReleaseAssetUrl -Url 'http://github.com/vishwassharma38/Akari-Sweeps/releases/download/v1.2.3/akari-sweeps-windows-x86_64.zip' -AssetName 'akari-sweeps-windows-x86_64.zip' -Tag 'v1.2.3'
    } 'HTTP downgrade must be rejected' | Out-Null
}

Test-Case 'asset redirects are restricted to GitHub and the approved release CDN hosts' {
    [void](Assert-AllowedRedirectUrl -Url 'https://release-assets.githubusercontent.com/asset?token=example' -AssetName $script:PackageName -Tag 'v1.2.3')
    [void](Assert-AllowedRedirectUrl -Url 'https://objects.githubusercontent.com/asset?token=example' -AssetName $script:PackageName -Tag 'v1.2.3')
    Assert-Throws {
        Assert-AllowedRedirectUrl -Url 'http://release-assets.githubusercontent.com/asset' -AssetName $script:PackageName -Tag 'v1.2.3'
    } 'HTTP CDN redirect must be rejected' | Out-Null
    Assert-Throws {
        Assert-AllowedRedirectUrl -Url 'https://attacker.example/asset' -AssetName $script:PackageName -Tag 'v1.2.3'
    } 'foreign redirect host must be rejected' | Out-Null
    Assert-Throws {
        Assert-AllowedRedirectUrl -Url 'https://github.com/other-owner/Akari-Sweeps/releases/download/v1.2.3/akari-sweeps-windows-x86_64.zip' -AssetName $script:PackageName -Tag 'v1.2.3'
    } 'redirect to a different repository must be rejected' | Out-Null
}

Test-Case 'limited stream writer rejects oversized downloads and removes partial files' {
    $root = New-TestRoot 'limited-stream'
    $stream = $null
    try {
        $destination = Join-Path $root 'oversized.bin'
        $stream = [IO.MemoryStream]::new([Text.Encoding]::UTF8.GetBytes('01234567890'))
        Assert-Throws {
            Write-LimitedStreamToFile -InputStream $stream -Destination $destination -MaxBytes 10 | Out-Null
        } 'oversized stream must be rejected' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath $destination)) 'partial oversized download was not removed'
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case 'safe ZIP extraction rejects traversal paths before writing outside the staging directory' {
    $root = New-TestRoot 'zip-traversal'
    try {
        $zip = Join-Path $root 'unsafe.zip'
        $stage = Join-Path $root 'stage'
        New-TestZipArchive -Path $zip -Entries @(
            @{ Name = '../escape.txt'; Content = 'outside' },
            @{ Name = 'akari-sweeps.exe'; Content = 'MZfake' }
        )
        Assert-Throws {
            Expand-SafePackage -PackagePath $zip -DestinationDirectory $stage
        } 'traversal entry must be rejected' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $root 'escape.txt'))) 'ZIP traversal wrote outside the staging directory'
        Assert-True (-not (Test-Path -LiteralPath $stage)) 'failed extraction left partial staging files'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case 'safe ZIP extraction rejects executable and script payloads inside assets' {
    $root = New-TestRoot 'zip-script'
    try {
        $zip = Join-Path $root 'unsafe.zip'
        $stage = Join-Path $root 'stage'
        New-TestZipArchive -Path $zip -Entries @(
            @{ Name = 'akari-sweeps.exe'; Content = 'MZfake' },
            @{ Name = 'assets/akari/sweep/frame.png'; Content = 'png' },
            @{ Name = 'assets/akari/sweep/payload.ps1'; Content = 'Write-Host unsafe' }
        )
        Assert-Throws {
            Expand-SafePackage -PackagePath $zip -DestinationDirectory $stage
        } 'script payload inside assets must be rejected' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath $stage)) 'rejected ZIP left partial staging files'
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case 'safe ZIP extraction enforces the archive entry-count ceiling' {
    $root = New-TestRoot 'zip-count-limit'
    $originalLimit = $script:MaxArchiveEntries
    try {
        $zip = Join-Path $root 'too-many.zip'
        $stage = Join-Path $root 'stage'
        New-TestZipArchive -Path $zip -Entries @(
            @{ Name = 'first.txt'; Content = 'one' },
            @{ Name = 'second.txt'; Content = 'two' }
        )
        $script:MaxArchiveEntries = 1
        Assert-Throws {
            Expand-SafePackage -PackagePath $zip -DestinationDirectory $stage
        } 'entry-count ceiling must be enforced' | Out-Null
        Assert-True (-not (Test-Path -LiteralPath $stage)) 'entry-count rejection left staging files'
    } finally {
        $script:MaxArchiveEntries = $originalLimit
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Test-Case 'failed asset commit rolls back every executable alias and the previous assets tree' {
    $root = New-TestRoot 'transaction-rollback'
    try {
        Reset-TestState
        $package = New-TestPackage -Root $root
        Set-MockDownload -PackagePath $package.Package -ChecksumPath $package.Checksum | Out-Null
        $install = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $install -Force | Out-Null

        foreach ($name in $script:AliasNames) {
            [IO.File]::WriteAllBytes((Join-Path $install $name), [byte[]](0x4D, 0x5A, 0x11, 0x22, 0x33))
        }
        foreach ($set in $script:RequiredSpriteSets) {
            $setDir = Join-Path $install ('assets/akari/' + $set)
            New-Item -ItemType Directory -Path $setDir -Force | Out-Null
            [IO.File]::WriteAllBytes((Join-Path $setDir 'frame_001.png'), [byte[]](9, 8, 7, 6))
        }
        [IO.File]::WriteAllText((Join-Path $install 'assets/old-version.txt'), 'previous assets')

        $oldHashes = @{}
        foreach ($name in $script:AliasNames) {
            $oldHashes[$name] = (Get-FileHash -LiteralPath (Join-Path $install $name) -Algorithm SHA256).Hash
        }
        $oldAssetHash = (Get-FileHash -LiteralPath (Join-Path $install 'assets/akari/sweep/frame_001.png') -Algorithm SHA256).Hash
        $script:FailMoveDestination = Join-Path $install 'assets'
        $script:FakeUserPath = 'C:\Windows;C:\Tools'

        Assert-Throws {
            Install-AkariSweeps -InstallDirectory $install
        } 'injected asset replacement failure must abort installation' | Out-Null

        foreach ($name in $script:AliasNames) {
            $actual = (Get-FileHash -LiteralPath (Join-Path $install $name) -Algorithm SHA256).Hash
            Assert-Equal $actual $oldHashes[$name] "$name was not restored after transaction rollback"
        }
        $actualAssetHash = (Get-FileHash -LiteralPath (Join-Path $install 'assets/akari/sweep/frame_001.png') -Algorithm SHA256).Hash
        Assert-Equal $actualAssetHash $oldAssetHash 'previous assets were not restored'
        Assert-Equal (Get-Content -LiteralPath (Join-Path $install 'assets/old-version.txt') -Raw) 'previous assets' 'previous asset tree was not restored'
        Assert-Equal $script:FakeUserPath 'C:\Windows;C:\Tools' 'PATH changed despite failed installation'
        Assert-True (-not (Get-ChildItem -LiteralPath $install -Force -Directory | Where-Object { $_.Name -like '.akari-install-backup-*' -or $_.Name -like '.akari-install-stage-*' })) 'successful rollback left transaction directories behind'
    } finally {
        $script:FailMoveDestination = $null
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($script:MockDownloadDir)) {
            Remove-Item -LiteralPath $script:MockDownloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Write-Host ''
if ($script:FailureCount -gt 0) {
    Write-Host ("$($script:FailureCount) of $($script:TestCount) tests failed.") -ForegroundColor Red
    exit 1
}
Write-Host ("All $($script:TestCount) installer tests passed.") -ForegroundColor Green
exit 0
