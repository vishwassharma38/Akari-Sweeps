<#
.SYNOPSIS
  Standalone Akari-Sweeps installer for Windows x86_64.

.DESCRIPTION
  Installs the already-compiled Akari-Sweeps release package. Rust, Cargo,
  and a source checkout are not required.

  The installer obtains the official GitHub release (latest by default, or an
  explicit -Version), downloads the Windows x86_64 ZIP and its published
  SHA-256 sidecar, verifies the archive, extracts it into an isolated staging
  directory, installs all six command aliases plus the complete assets tree,
  and adds the install directory to the current user's PATH.

  The installation directory defaults to %LOCALAPPDATA%\Akari-Sweeps and does
  not require administrator privileges. Unrelated files in an existing install
  directory are preserved; only the six aliases and the application-owned
  assets directory are replaced.

.PARAMETER InstallDir
  Installation directory. Defaults to %LOCALAPPDATA%\Akari-Sweeps.

.PARAMETER Version
  Optional release version, for example 0.1.0 or v0.1.0. When omitted, the
  latest non-prerelease GitHub release is used.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\install-release.ps1

.EXAMPLE
  .\install-release.ps1 -Version 0.1.1 -InstallDir "D:\Tools\Akari"
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Akari-Sweeps'),
    [string]$Version
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:RepositoryOwner = 'vishwassharma38'
$script:RepositoryName = 'Akari-Sweeps'
$script:Repository = $script:RepositoryOwner + '/' + $script:RepositoryName
$script:LatestReleaseApi = 'https://api.github.com/repos/' + $script:Repository + '/releases/latest'
$script:ReleaseDownloadBase = 'https://github.com/' + $script:Repository + '/releases/download'
$script:PackageName = 'akari-sweeps-windows-x86_64.zip'
$script:ChecksumName = 'akari-sweeps-windows-x86_64.zip.sha256'
$script:UserAgent = 'Akari-Sweeps-Standalone-Installer'
$script:AliasNames = @(
    'akari.exe',
    'akariclean.exe',
    'akarihealth.exe',
    'akariall.exe',
    'akariabout.exe',
    'akarihelp.exe'
)
$script:RequiredSpriteSets = @('sweep', 'idle', 'health', 'about', 'help')

function Write-Step {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Green
}

function Get-NormalizedTag {
    param([string]$VersionText)
    if ([string]::IsNullOrWhiteSpace($VersionText)) {
        throw 'A version was requested but no value was supplied.'
    }
    $value = $VersionText.Trim()
    if ($value.StartsWith('v') -or $value.StartsWith('V')) {
        $value = $value.Substring(1)
    }
    if ($value -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$') {
        throw "Invalid version '$VersionText'. Expected a semantic version such as 0.1.1."
    }
    return 'v' + $value
}

function Enable-ModernTls {
    try {
        $current = [Net.ServicePointManager]::SecurityProtocol
        [Net.ServicePointManager]::SecurityProtocol = $current -bor [Net.SecurityProtocolType]::Tls12
    } catch {
        # Windows PowerShell 5.1 on supported Windows versions already
        # supports TLS 1.2; this is best-effort for older configurations.
    }
}

function Get-LatestReleaseTag {
    Enable-ModernTls
    $headers = @{
        'User-Agent' = $script:UserAgent
        'Accept' = 'application/vnd.github+json'
    }
    $release = Invoke-RestMethod -Uri $script:LatestReleaseApi -Headers $headers -UseBasicParsing
    $tag = [string]$release.tag_name
    if ([string]::IsNullOrWhiteSpace($tag)) {
        throw "GitHub did not return a release tag for $($script:Repository)."
    }
    return (Get-NormalizedTag $tag)
}

function Get-RemoteFile {
    param(
        [string]$Uri,
        [string]$Destination
    )
    Enable-ModernTls
    $headers = @{ 'User-Agent' = $script:UserAgent }
    Invoke-WebRequest -Uri $Uri -OutFile $Destination -Headers $headers -UseBasicParsing
}

function Read-ChecksumFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "The release checksum file '$Path' is missing."
    }
    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    $line = @($text -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1)
    if ($line.Count -eq 0) {
        throw 'The release checksum file is empty.'
    }
    $hash = ($line[0].Trim() -split '\s+')[0]
    if ($hash -notmatch '^[0-9A-Fa-f]{64}$') {
        throw 'The release checksum file does not contain a valid SHA-256 value.'
    }
    return $hash.ToLowerInvariant()
}

function Assert-PackageChecksum {
    param(
        [string]$PackagePath,
        [string]$ChecksumPath
    )
    if (-not (Test-Path -LiteralPath $PackagePath -PathType Leaf)) {
        throw "The release package '$PackagePath' is missing."
    }
    $expected = Read-ChecksumFile $ChecksumPath
    $actual = (Get-FileHash -LiteralPath $PackagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "SHA-256 mismatch: the release checksum says $expected but the downloaded package is $actual."
    }
}

function Assert-PackageLayout {
    param([string]$ExtractDir)
    $exe = Join-Path $ExtractDir 'akari-sweeps.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw "The release package is missing 'akari-sweeps.exe'."
    }
    $stream = $null
    try {
        $stream = [IO.File]::OpenRead($exe)
        $header = New-Object byte[] 2
        $read = $stream.Read($header, 0, 2)
        if ($read -ne 2 -or $header[0] -ne 0x4D -or $header[1] -ne 0x5A) {
            throw "The packaged 'akari-sweeps.exe' is not a Windows executable."
        }
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }

    $assets = Join-Path $ExtractDir 'assets'
    if (-not (Test-Path -LiteralPath $assets -PathType Container)) {
        throw "The release package is missing the 'assets' directory."
    }
    foreach ($set in $script:RequiredSpriteSets) {
        $setDir = Join-Path $assets ('akari\' + $set)
        if (-not (Test-Path -LiteralPath $setDir -PathType Container)) {
            throw "The release package is missing the '$set' sprite directory."
        }
        $pngs = @(Get-ChildItem -LiteralPath $setDir -File -Filter '*.png' -ErrorAction SilentlyContinue)
        if ($pngs.Count -eq 0) {
            throw "The release package contains no PNG frames in the '$set' sprite directory."
        }
    }
}

function Get-InstallerUserPath {
    return [Environment]::GetEnvironmentVariable('Path', 'User')
}

function Set-InstallerUserPath {
    param([string]$PathValue)
    [Environment]::SetEnvironmentVariable('Path', $PathValue, 'User')
}

function Add-InstallDirToUserPath {
    param([string]$PathToAdd)
    $normalized = $PathToAdd.TrimEnd([char[]]@('\', '/'))
    $current = Get-InstallerUserPath
    $parts = @()
    if (-not [string]::IsNullOrWhiteSpace($current)) {
        $parts = @($current -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    }
    foreach ($part in $parts) {
        $candidate = $part.Trim().TrimEnd([char[]]@('\', '/'))
        if ([string]::Equals($candidate, $normalized, [StringComparison]::OrdinalIgnoreCase)) {
            return $false
        }
    }
    if ($parts.Count -eq 0) {
        Set-InstallerUserPath $normalized
    } else {
        Set-InstallerUserPath (($parts -join ';') + ';' + $normalized)
    }
    return $true
}

function Assert-InstallDirectoryIsSafe {
    param([string]$Path)
    $full = [IO.Path]::GetFullPath($Path).TrimEnd([char[]]@('\', '/'))
    if ($full -match '^[A-Za-z]:$') {
        throw "Refusing to install into a drive root: '$full'."
    }
    $windows = [IO.Path]::GetFullPath($env:WINDIR).TrimEnd([char[]]@('\', '/'))
    if ([string]::Equals($full, $windows, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to install into the Windows directory: '$full'."
    }
    return $full
}

function Assert-NoRunningAliases {
    param([string]$InstallDirectory)
    $blocked = @()
    foreach ($name in $script:AliasNames) {
        $path = Join-Path $InstallDirectory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            continue
        }
        $stream = $null
        try {
            $stream = [IO.File]::Open(
                $path,
                [IO.FileMode]::Open,
                [IO.FileAccess]::ReadWrite,
                [IO.FileShare]::None
            )
        } catch {
            $blocked += $name
        } finally {
            if ($null -ne $stream) {
                $stream.Dispose()
            }
        }
    }
    if ($blocked.Count -gt 0) {
        throw ("Close Akari-Sweeps before installing. These installed files are in use: " +
            ($blocked -join ', ') + '.')
    }
}

function Install-FromVerifiedPackage {
    param(
        [string]$PackagePath,
        [string]$ChecksumPath,
        [string]$InstallDirectory
    )
    Assert-PackageChecksum -PackagePath $PackagePath -ChecksumPath $ChecksumPath
    $target = Assert-InstallDirectoryIsSafe $InstallDirectory

    $extractDir = Join-Path ([IO.Path]::GetTempPath()) ('akari-sweeps-extract-' + [Guid]::NewGuid().ToString('N'))
    $pendingTemp = New-Object System.Collections.Generic.List[string]
    try {
        New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
        Expand-Archive -LiteralPath $PackagePath -DestinationPath $extractDir -Force
        Assert-PackageLayout $extractDir

        New-Item -ItemType Directory -Path $target -Force | Out-Null
        Assert-NoRunningAliases $target

        $canonical = Join-Path $extractDir 'akari-sweeps.exe'
        $canonicalHash = (Get-FileHash -LiteralPath $canonical -Algorithm SHA256).Hash.ToLowerInvariant()
        foreach ($name in $script:AliasNames) {
            $destination = Join-Path $target $name
            $temp = Join-Path $target ('.' + $name + '.new-' + [Guid]::NewGuid().ToString('N'))
            [void]$pendingTemp.Add($temp)
            Copy-Item -LiteralPath $canonical -Destination $temp -Force
            $tempHash = (Get-FileHash -LiteralPath $temp -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($tempHash -ne $canonicalHash) {
                throw "The prepared copy of '$name' failed SHA-256 verification."
            }
            Move-Item -LiteralPath $temp -Destination $destination -Force
            [void]$pendingTemp.Remove($temp)
        }

        $stagedAssets = Join-Path $extractDir 'assets'
        $installAssets = Join-Path $target 'assets'
        $newAssets = Join-Path $target ('.assets.new-' + [Guid]::NewGuid().ToString('N'))
        [void]$pendingTemp.Add($newAssets)
        Copy-Item -LiteralPath $stagedAssets -Destination $newAssets -Recurse -Force

        if (Test-Path -LiteralPath $installAssets) {
            $oldAssets = Join-Path $target ('.assets.old-' + [Guid]::NewGuid().ToString('N'))
            [void]$pendingTemp.Add($oldAssets)
            Move-Item -LiteralPath $installAssets -Destination $oldAssets
            try {
                Move-Item -LiteralPath $newAssets -Destination $installAssets
            } catch {
                Move-Item -LiteralPath $oldAssets -Destination $installAssets
                throw
            }
            [void]$pendingTemp.Remove($newAssets)
            Remove-Item -LiteralPath $oldAssets -Recurse -Force
            [void]$pendingTemp.Remove($oldAssets)
        } else {
            Move-Item -LiteralPath $newAssets -Destination $installAssets
            [void]$pendingTemp.Remove($newAssets)
        }

        $pathAdded = Add-InstallDirToUserPath $target
        return [pscustomobject]@{
            InstallDirectory = $target
            PathAdded = $pathAdded
        }
    } finally {
        foreach ($temp in @($pendingTemp)) {
            if (Test-Path -LiteralPath $temp) {
                Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
        if (Test-Path -LiteralPath $extractDir) {
            Remove-Item -LiteralPath $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

function Install-AkariSweeps {
    param(
        [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Akari-Sweeps'),
        [string]$RequestedVersion = $null
    )
    $architecture = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrWhiteSpace($architecture)) {
        $architecture = $env:PROCESSOR_ARCHITECTURE
    }
    if (-not [string]::IsNullOrWhiteSpace($architecture) -and $architecture -ne 'AMD64') {
        throw "The Akari-Sweeps release package supports Windows x86_64 only (detected '$architecture')."
    }

    $tag = $null
    if ([string]::IsNullOrWhiteSpace($RequestedVersion)) {
        Write-Step 'Finding the latest Akari-Sweeps release...'
        $tag = Get-LatestReleaseTag
    } else {
        $tag = Get-NormalizedTag $RequestedVersion
    }

    $packageUrl = $script:ReleaseDownloadBase + '/' + $tag + '/' + $script:PackageName
    $checksumUrl = $script:ReleaseDownloadBase + '/' + $tag + '/' + $script:ChecksumName

    $downloadDir = Join-Path ([IO.Path]::GetTempPath()) ('akari-sweeps-install-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $downloadDir -Force | Out-Null
        $packagePath = Join-Path $downloadDir $script:PackageName
        $checksumPath = Join-Path $downloadDir $script:ChecksumName

        Write-Step "Downloading Akari-Sweeps $tag..."
        Get-RemoteFile -Uri $packageUrl -Destination $packagePath
        Get-RemoteFile -Uri $checksumUrl -Destination $checksumPath

        Write-Step 'Verifying the release package (SHA-256)...'
        $result = Install-FromVerifiedPackage -PackagePath $packagePath -ChecksumPath $checksumPath -InstallDirectory $InstallDirectory
    } finally {
        if (Test-Path -LiteralPath $downloadDir) {
            Remove-Item -LiteralPath $downloadDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    Write-Success ''
    Write-Success "Akari-Sweeps $tag installed to $($result.InstallDirectory)"
    Write-Host 'Installed commands: akari, akariclean, akarihealth, akariall, akariabout, akarihelp'
    if ($result.PathAdded) {
        Write-Host "Added to the current user's PATH: $($result.InstallDirectory)" -ForegroundColor Green
        Write-Host 'Restart your terminal (or sign out and back in) before using the commands.'
    } else {
        Write-Host "The current user's PATH already contains $($result.InstallDirectory)."
    }
    Write-Host 'Verify with: akari --version'
    Write-Host 'Uninstall with uninstall.ps1 from the repository (see README).'
}

# Dot-sourcing is used by tests to load the functions without installing.
if ($MyInvocation.InvocationName -ne '.') {
    try {
        Install-AkariSweeps -InstallDirectory $InstallDir -RequestedVersion $Version
        exit 0
    } catch {
        Write-Host ''
        Write-Host "Akari-Sweeps installation failed: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
}
