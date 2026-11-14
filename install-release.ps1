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

# Use .NET directly rather than relying on Get-FileHash being discoverable
# through PowerShell module auto-loading in the parent/child process environment.
function Get-Sha256Hash {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string]$LiteralPath
    )

    $stream = $null
    $algorithm = $null
    try {
        $stream = [IO.File]::OpenRead($LiteralPath)
        $algorithm = [Security.Cryptography.SHA256]::Create()
        $digest = $algorithm.ComputeHash($stream)
        return ([BitConverter]::ToString($digest)).Replace('-', '').ToLowerInvariant()
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
        if ($null -ne $algorithm) {
            $algorithm.Dispose()
        }
    }
}


$ProgressPreference = 'SilentlyContinue'

$script:RepositoryOwner = 'vishwassharma38'
$script:RepositoryName = 'Akari-Sweeps'
$script:Repository = $script:RepositoryOwner + '/' + $script:RepositoryName
$script:LatestReleaseApi = 'https://api.github.com/repos/' + $script:Repository + '/releases/latest'
$script:ReleaseDownloadBase = 'https://github.com/' + $script:Repository + '/releases/download'
$script:PackageName = 'akari-sweeps-windows-x86_64.zip'
$script:ChecksumName = 'akari-sweeps-windows-x86_64.zip.sha256'
$script:UserAgent = 'Akari-Sweeps-Standalone-Installer'
$script:MaxCompressedDownloadBytes = 100 * 1024 * 1024
$script:MaxChecksumDownloadBytes = 64 * 1024
$script:MaxArchiveEntryBytes = 512 * 1024 * 1024
$script:MaxArchiveTotalBytes = 1024 * 1024 * 1024
$script:MaxArchiveEntries = 10000
$script:MaxRedirects = 5
$script:AllowedAssetRedirectHosts = @(
    'release-assets.githubusercontent.com',
    'objects.githubusercontent.com'
)
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

function Assert-ReleaseAssetUrl {
    param(
        [string]$Url,
        [string]$AssetName,
        [string]$Tag
    )
    $uri = $null
    if (-not [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref]$uri)) {
        throw "The release asset URL is invalid: '$Url'."
    }
    if ($uri.Scheme -ne 'https' -or $uri.Host -ne 'github.com' -or
        -not $uri.IsDefaultPort -or -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw "The release asset URL is not an approved HTTPS GitHub URL: '$Url'."
    }

    $segments = @($uri.AbsolutePath.Trim('/').Split('/') | ForEach-Object {
        [Uri]::UnescapeDataString($_)
    })
    if ($segments.Count -ne 6 -or
        $segments[0] -cne $script:RepositoryOwner -or
        $segments[1] -cne $script:RepositoryName -or
        $segments[2] -cne 'releases' -or
        $segments[3] -cne 'download' -or
        $segments[4] -cne $Tag -or
        $segments[5] -cne $AssetName) {
        throw "The release asset URL does not identify the expected repository, tag, and asset: '$Url'."
    }
    return $uri
}

function Assert-AllowedRedirectUrl {
    param(
        [string]$Url,
        [string]$AssetName,
        [string]$Tag
    )
    $uri = $null
    if (-not [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref]$uri)) {
        throw 'GitHub returned an invalid asset redirect URL.'
    }
    if ($uri.Scheme -ne 'https' -or -not $uri.IsDefaultPort -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw "The asset redirect is not an approved HTTPS URL: '$Url'."
    }

    if ($uri.Host -eq 'github.com') {
        return (Assert-ReleaseAssetUrl -Url $Url -AssetName $AssetName -Tag $Tag)
    }
    if ($script:AllowedAssetRedirectHosts -notcontains $uri.Host.ToLowerInvariant()) {
        throw "The asset redirect host '$($uri.Host)' is not approved."
    }
    return $uri
}

function Write-LimitedStreamToFile {
    param(
        [IO.Stream]$InputStream,
        [string]$Destination,
        [long]$MaxBytes
    )
    $output = $null
    $written = [long]0
    try {
        $output = [IO.File]::Open(
            $Destination,
            [IO.FileMode]::Create,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $buffer = New-Object byte[] 65536
        while (($read = $InputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $written += $read
            if ($written -gt $MaxBytes) {
                throw "The downloaded file exceeds the $MaxBytes-byte size limit."
            }
            $output.Write($buffer, 0, $read)
        }
        $output.Flush()
    } catch {
        if ($null -ne $output) {
            $output.Dispose()
            $output = $null
        }
        if (Test-Path -LiteralPath $Destination) {
            Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        }
        throw
    } finally {
        if ($null -ne $output) {
            $output.Dispose()
        }
    }
    return $written
}

function Get-RemoteFile {
    param(
        [string]$Uri,
        [string]$Destination
    )
    Enable-ModernTls
    $initialUri = $null
    if (-not [Uri]::TryCreate($Uri, [UriKind]::Absolute, [ref]$initialUri)) {
        throw "The release asset URL is invalid: '$Uri'."
    }
    $segments = @($initialUri.AbsolutePath.Trim('/').Split('/') | ForEach-Object {
        [Uri]::UnescapeDataString($_)
    })
    if ($segments.Count -ne 6) {
        throw 'The release asset URL has an unexpected path.'
    }
    $assetName = [IO.Path]::GetFileName($segments[5])
    $tag = $segments[4]
    $current = Assert-ReleaseAssetUrl -Url $Uri -AssetName $assetName -Tag $tag

    $maxBytes = [long]$script:MaxCompressedDownloadBytes
    if ($assetName -eq $script:ChecksumName) {
        $maxBytes = [long]$script:MaxChecksumDownloadBytes
    }

    for ($hop = 0; $hop -le $script:MaxRedirects; $hop++) {
        $request = [Net.HttpWebRequest][Net.WebRequest]::Create($current)
        $request.AllowAutoRedirect = $false
        $request.Method = 'GET'
        $request.UserAgent = $script:UserAgent
        $request.Accept = 'application/octet-stream'
        $request.Timeout = 600000
        $request.ReadWriteTimeout = 30000
        $response = $null
        try {
            try {
                $response = $request.GetResponse()
            } catch [Net.WebException] {
                if ($null -ne $_.Exception.Response) {
                    $status = [int]$_.Exception.Response.StatusCode
                    $_.Exception.Response.Dispose()
                    throw "The download of '$assetName' returned HTTP $status."
                }
                throw
            }

            $status = [int]$response.StatusCode
            if ($status -in @(301, 302, 303, 307, 308)) {
                if ($hop -ge $script:MaxRedirects) {
                    throw "The download of '$assetName' exceeded the $($script:MaxRedirects)-redirect limit."
                }
                $location = $response.Headers['Location']
                if ([string]::IsNullOrWhiteSpace($location)) {
                    throw "The redirect for '$assetName' did not include a Location header."
                }
                $next = [Uri]::new($current, $location)
                $current = Assert-AllowedRedirectUrl -Url $next.AbsoluteUri -AssetName $assetName -Tag $tag
                continue
            }
            if ($status -ne 200) {
                throw "The download of '$assetName' returned unexpected HTTP status $status."
            }
            if ($response.ContentLength -gt $maxBytes) {
                throw "The downloaded '$assetName' exceeds the $maxBytes-byte size limit."
            }

            $inputStream = $response.GetResponseStream()
            try {
                [void](Write-LimitedStreamToFile -InputStream $inputStream -Destination $Destination -MaxBytes $maxBytes)
            } finally {
                if ($null -ne $inputStream) {
                    $inputStream.Dispose()
                }
            }
            return
        } finally {
            if ($null -ne $response) {
                $response.Dispose()
            }
        }
    }
    throw "The download of '$assetName' could not complete within the redirect limit."
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
    $actual = Get-Sha256Hash -LiteralPath $PackagePath
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

function Get-ValidatedArchiveEntryPath {
    param(
        [string]$EntryName,
        [bool]$IsDirectory
    )
    if ([string]::IsNullOrEmpty($EntryName) -or $EntryName.Contains([char]0)) {
        throw "The ZIP entry '$EntryName' is empty or contains a NUL character."
    }
    $normalized = $EntryName.Replace('\', '/')
    if ($normalized.StartsWith('/')) {
        throw "The ZIP entry '$EntryName' uses an absolute path."
    }
    if (-not $IsDirectory -and $normalized.EndsWith('/')) {
        throw "The ZIP file entry '$EntryName' has a trailing separator."
    }
    $trimmed = $normalized.TrimEnd('/')
    if ([string]::IsNullOrEmpty($trimmed)) {
        throw "The ZIP entry '$EntryName' has no file name."
    }

    $parts = @($trimmed.Split('/'))
    foreach ($part in $parts) {
        if ([string]::IsNullOrEmpty($part) -or $part -eq '.' -or $part -eq '..') {
            throw "The ZIP entry '$EntryName' contains an unsafe path component."
        }
        if ($part.Contains(':') -or $part.EndsWith(' ') -or $part.EndsWith('.') -or
            $part.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0) {
            throw "The ZIP entry '$EntryName' contains a Windows-unsafe path component."
        }
        foreach ($character in $part.ToCharArray()) {
            if ([char]::IsControl($character)) {
                throw "The ZIP entry '$EntryName' contains a control character."
            }
        }
    }

    $lower = @($parts | ForEach-Object { $_.ToLowerInvariant() })
    $key = $lower -join '/'
    if ($IsDirectory) {
        if ($key -ne 'assets' -and -not $key.StartsWith('assets/')) {
            throw "The ZIP directory '$EntryName' is not part of the application package."
        }
        if ($key -eq 'assets') {
            return [pscustomobject]@{
                Key = $key; RelativePath = 'assets'; IsCanonical = $false; IsDirectory = $true
            }
        }
        $relative = Join-Path 'assets' (($parts | Select-Object -Skip 1) -join [IO.Path]::DirectorySeparatorChar)
        return [pscustomobject]@{
            Key = $key; RelativePath = $relative; IsCanonical = $false; IsDirectory = $true
        }
    }

    if ($key -eq 'akari-sweeps.exe') {
        return [pscustomobject]@{
            Key = $key; RelativePath = 'akari-sweeps.exe'; IsCanonical = $true; IsDirectory = $false
        }
    }
    if ($key -eq 'assets') {
        throw "The ZIP entry '$EntryName' must be a directory."
    }
    if (-not $key.StartsWith('assets/')) {
        throw "The ZIP file '$EntryName' is not part of the application package."
    }
    $fileName = $parts[$parts.Count - 1].ToLowerInvariant()
    if ($fileName -match '\.(exe|dll|bat|cmd|com|scr|ps1|msi|vbs|vbe|js|jse|wsf|wsh|cpl|pif|hta|lnk)$') {
        throw "The ZIP file '$EntryName' is an unexpected executable or script inside assets."
    }
    $relative = Join-Path 'assets' (($parts | Select-Object -Skip 1) -join [IO.Path]::DirectorySeparatorChar)
    return [pscustomobject]@{
        Key = $key; RelativePath = $relative; IsCanonical = $false; IsDirectory = $false
    }
}

function Copy-ZipEntryWithLimits {
    param(
        [IO.Compression.ZipArchiveEntry]$Entry,
        [string]$Destination,
        [ref]$TotalBytes
    )
    $inputStream = $null
    $outputStream = $null
    $entryBytes = [long]0
    try {
        $inputStream = $Entry.Open()
        $outputStream = [IO.File]::Open(
            $Destination,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $buffer = New-Object byte[] 65536
        while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $entryBytes += $read
            $TotalBytes.Value += $read
            if ($entryBytes -gt $script:MaxArchiveEntryBytes) {
                throw "The ZIP entry '$($Entry.FullName)' exceeds the per-entry extraction limit."
            }
            if ($TotalBytes.Value -gt $script:MaxArchiveTotalBytes) {
                throw 'The ZIP archive expands beyond the total extraction size limit.'
            }
            $outputStream.Write($buffer, 0, $read)
        }
        $outputStream.Flush()
    } finally {
        if ($null -ne $outputStream) { $outputStream.Dispose() }
        if ($null -ne $inputStream) { $inputStream.Dispose() }
    }
}

function Expand-SafePackage {
    param(
        [string]$PackagePath,
        [string]$DestinationDirectory
    )
    $archive = $null
    $success = $false
    try {
        Add-Type -AssemblyName System.IO.Compression
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
        $archive = [IO.Compression.ZipFile]::OpenRead($PackagePath)
        if ($archive.Entries.Count -gt $script:MaxArchiveEntries) {
            throw "The ZIP archive contains more than $($script:MaxArchiveEntries) entries."
        }

        $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        $totalDeclaredBytes = [long]0
        $totalWrittenBytes = [long]0
        foreach ($entry in $archive.Entries) {
            $isDirectory = [string]::IsNullOrEmpty($entry.Name) -or
                $entry.FullName.EndsWith('/') -or $entry.FullName.EndsWith('\')
            $unixMode = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($unixMode -eq 0xA000) {
                throw "The ZIP entry '$($entry.FullName)' is a symbolic link, which is not allowed."
            }

            $validated = Get-ValidatedArchiveEntryPath -EntryName $entry.FullName -IsDirectory $isDirectory
            if (-not $seen.Add($validated.Key)) {
                throw "The ZIP archive contains a duplicate entry '$($entry.FullName)'."
            }

            $destination = Join-Path $DestinationDirectory $validated.RelativePath
            if ($isDirectory) {
                New-Item -ItemType Directory -Path $destination -Force | Out-Null
                continue
            }
            if ($entry.Length -gt $script:MaxArchiveEntryBytes) {
                throw "The ZIP entry '$($entry.FullName)' exceeds the per-entry extraction limit."
            }
            $totalDeclaredBytes += [long]$entry.Length
            if ($totalDeclaredBytes -gt $script:MaxArchiveTotalBytes) {
                throw 'The ZIP archive declares more than the total extraction size limit.'
            }
            if ($validated.IsCanonical -and $entry.Length -lt 2) {
                throw "The packaged executable '$($script:PackageName)' is empty."
            }

            $parent = Split-Path -Parent $destination
            if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
                New-Item -ItemType Directory -Path $parent -Force | Out-Null
            }
            [void](Copy-ZipEntryWithLimits -Entry $entry -Destination $destination -TotalBytes ([ref]$totalWrittenBytes))
        }

        Assert-PackageLayout -ExtractDir $DestinationDirectory
        $success = $true
    } catch {
        if (Test-Path -LiteralPath $DestinationDirectory) {
            Remove-Item -LiteralPath $DestinationDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        throw
    } finally {
        if ($null -ne $archive) { $archive.Dispose() }
        if (-not $success -and (Test-Path -LiteralPath $DestinationDirectory)) {
            Remove-Item -LiteralPath $DestinationDirectory -Recurse -Force -ErrorAction SilentlyContinue
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
    if ([string]::Equals($full, $windows, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($windows + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to install inside the Windows directory: '$full'."
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

function Move-InstallerItem {
    param(
        [string]$Source,
        [string]$Destination
    )
    Move-Item -LiteralPath $Source -Destination $Destination -Force
}

function Assert-ManagedDestinationIsSafe {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to replace a managed installation path that is a reparse point: '$Path'."
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
    $stageDir = $null
    $backupDir = $null
    $operations = New-Object System.Collections.ArrayList
    $rollbackErrors = New-Object System.Collections.ArrayList
    $installSucceeded = $false
    $rollbackComplete = $true
    $originalUserPath = Get-InstallerUserPath

    try {
        Expand-SafePackage -PackagePath $PackagePath -DestinationDirectory $extractDir
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        Assert-NoRunningAliases $target

        $stageDir = Join-Path $target ('.akari-install-stage-' + [Guid]::NewGuid().ToString('N'))
        $backupDir = Join-Path $target ('.akari-install-backup-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
        New-Item -ItemType Directory -Path $backupDir -Force | Out-Null

        $canonical = Join-Path $extractDir 'akari-sweeps.exe'
        $canonicalHash = Get-Sha256Hash -LiteralPath $canonical
        foreach ($name in $script:AliasNames) {
            $stagedAlias = Join-Path $stageDir $name
            Copy-Item -LiteralPath $canonical -Destination $stagedAlias
            $stagedHash = Get-Sha256Hash -LiteralPath $stagedAlias
            if ($stagedHash -ne $canonicalHash) {
                throw "The staged copy of '$name' failed SHA-256 verification."
            }
            [void]$operations.Add([pscustomobject]@{
                Name = $name
                Source = $stagedAlias
                Destination = (Join-Path $target $name)
                Backup = (Join-Path $backupDir $name)
                BackedUp = $false
                Installed = $false
            })
        }

        $stagedAssets = Join-Path $stageDir 'assets'
        Copy-Item -LiteralPath (Join-Path $extractDir 'assets') -Destination $stageDir -Recurse
        [void]$operations.Add([pscustomobject]@{
            Name = 'assets'
            Source = $stagedAssets
            Destination = (Join-Path $target 'assets')
            Backup = (Join-Path $backupDir 'assets')
            BackedUp = $false
            Installed = $false
        })

        foreach ($operation in $operations) {
            Assert-ManagedDestinationIsSafe -Path $operation.Destination
            if (Test-Path -LiteralPath $operation.Destination) {
                Move-InstallerItem -Source $operation.Destination -Destination $operation.Backup
                $operation.BackedUp = $true
            }
            Move-InstallerItem -Source $operation.Source -Destination $operation.Destination
            $operation.Installed = $true
        }

        $pathAdded = Add-InstallDirToUserPath $target
        $installSucceeded = $true
        return [pscustomobject]@{
            InstallDirectory = $target
            PathAdded = $pathAdded
        }
    } catch {
        $installError = $_
        for ($index = $operations.Count - 1; $index -ge 0; $index--) {
            $operation = $operations[$index]
            try {
                if ($operation.Installed -and (Test-Path -LiteralPath $operation.Destination)) {
                    Remove-Item -LiteralPath $operation.Destination -Recurse -Force
                }
                if ($operation.BackedUp -and (Test-Path -LiteralPath $operation.Backup)) {
                    Move-InstallerItem -Source $operation.Backup -Destination $operation.Destination
                }
            } catch {
                $rollbackComplete = $false
                [void]$rollbackErrors.Add("$($operation.Name): $($_.Exception.Message)")
            }
        }

        try {
            $currentUserPath = Get-InstallerUserPath
            if ($currentUserPath -cne $originalUserPath) {
                Set-InstallerUserPath $originalUserPath
            }
        } catch {
            $rollbackComplete = $false
            [void]$rollbackErrors.Add("user PATH: $($_.Exception.Message)")
        }

        if (-not $rollbackComplete) {
            $details = @($rollbackErrors) -join '; '
            throw "Installation failed: $($installError.Exception.Message). Rollback was incomplete ($details). Previous files were preserved where possible in '$backupDir'."
        }
        throw $installError
    } finally {
        if ($null -ne $stageDir -and (Test-Path -LiteralPath $stageDir)) {
            Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        if ($installSucceeded -and $null -ne $backupDir -and (Test-Path -LiteralPath $backupDir)) {
            try {
                Remove-Item -LiteralPath $backupDir -Recurse -Force
            } catch {
                Write-Warning "Installation succeeded, but the previous-version backup could not be removed: '$backupDir'."
            }
        } elseif ($rollbackComplete -and $null -ne $backupDir -and (Test-Path -LiteralPath $backupDir)) {
            Remove-Item -LiteralPath $backupDir -Recurse -Force -ErrorAction SilentlyContinue
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

    $encodedTag = [Uri]::EscapeDataString($tag)
    $packageUrl = $script:ReleaseDownloadBase + '/' + $encodedTag + '/' + $script:PackageName
    $checksumUrl = $script:ReleaseDownloadBase + '/' + $encodedTag + '/' + $script:ChecksumName

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
