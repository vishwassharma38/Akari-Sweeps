<#
.SYNOPSIS
  Transactional Akari-Sweeps update helper.

.DESCRIPTION
  Launched by the Rust updater after the release ZIP has been verified with
  SHA-256 and extracted into an isolated staging directory. This helper waits
  for the relevant Akari-Sweeps processes to exit, backs up the installation,
  replaces all six executable aliases and the assets directory, verifies the
  result, and rolls back if anything fails.

  Every path is supplied as a separate process argument by the Rust updater.
  This script never receives paths interpolated into command source.

  Exit codes:
    0  update completed and verified
    1  update failed; the previous installation was restored (or untouched)
    2  update failed and rollback also failed; recovery files were retained
    3  timed out waiting for running Akari-Sweeps processes to exit
#>
param(
    [Parameter(Mandatory = $true)][string]$InstallDir,
    [Parameter(Mandatory = $true)][string]$StagingDir,
    [Parameter(Mandatory = $true)][string]$ManifestPath,
    [Parameter(Mandatory = $true)][string]$BackupDir,
    [Parameter(Mandatory = $true)][string]$LogPath,
    [int]$WaitSeconds = 120,
    # Automated-test fault injection only. The Rust updater never passes it.
    [switch]$TestFailAfterAliasesReplace
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$script:LogPath = $LogPath
$script:ScriptPath = $PSCommandPath
if ([string]::IsNullOrEmpty($script:ScriptPath)) {
    $script:ScriptPath = $MyInvocation.MyCommand.Path
}
$script:AliasNames = @(
    'akari.exe',
    'akariclean.exe',
    'akarihealth.exe',
    'akariall.exe',
    'akariabout.exe',
    'akarihelp.exe'
)
$script:ReplacementStarted = $false
$script:Committed = $false
$script:PathsValidated = $false
$script:AssetsTouched = $false
$script:AssetsExisted = $false
$script:AliasesExisted = @{}
$script:PendingTemps = New-Object System.Collections.Generic.List[string]

function Write-Log {
    param(
        [string]$Message,
        [string]$Level = 'INFO'
    )
    $line = ('{0:yyyy-MM-dd HH:mm:ss} [{1}] {2}' -f (Get-Date), $Level, $Message)
    try {
        $parent = [IO.Path]::GetDirectoryName($script:LogPath)
        if (-not [string]::IsNullOrEmpty($parent)) {
            [IO.Directory]::CreateDirectory($parent) | Out-Null
        }
        $encoding = New-Object System.Text.UTF8Encoding($false)
        [IO.File]::AppendAllText($script:LogPath, $line + [Environment]::NewLine, $encoding)
    } catch {
        # Logging must never turn a safe failure into an unsafe success.
    }
    try {
        Write-Host $line
    } catch {
    }
}

function Get-TrimmedFullPath {
    param([string]$Path)
    return [IO.Path]::GetFullPath($Path).TrimEnd([char[]]@('\', '/'))
}

function Test-PathWithin {
    param(
        [string]$Child,
        [string]$Parent
    )
    $childFull = Get-TrimmedFullPath $Child
    $parentFull = Get-TrimmedFullPath $Parent
    if ([string]::Equals($childFull, $parentFull, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $prefix = $parentFull + [IO.Path]::DirectorySeparatorChar
    return $childFull.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Remove-SafePath {
    param([string]$Path)
    if ([string]::IsNullOrEmpty($Path)) {
        return
    }
    try {
        if (Test-Path -LiteralPath $Path -PathType Container) {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
        } elseif (Test-Path -LiteralPath $Path) {
            Remove-Item -LiteralPath $Path -Force -ErrorAction Stop
        }
    } catch {
        Write-Log "Could not remove temporary path '$Path': $($_.Exception.Message)" 'WARN'
    }
}

function Remove-TransactionFiles {
    if (-not $script:PathsValidated) {
        return
    }
    $workspace = $null
    if (-not [string]::IsNullOrEmpty($script:StagingDir)) {
        $workspace = Split-Path -Parent $script:StagingDir
    }
    foreach ($temp in @($script:PendingTemps)) {
        Remove-SafePath $temp
    }
    $script:PendingTemps.Clear()
    Remove-SafePath $script:StagingDir
    Remove-SafePath $script:ManifestPath
    Remove-SafePath $script:BackupDir
    if (-not [string]::IsNullOrEmpty($workspace)) {
        Remove-SafePath $workspace
    }
}

function Assert-TreeMatches {
    param(
        [string]$ExpectedRoot,
        [string]$ActualRoot
    )
    if (-not (Test-Path -LiteralPath $ExpectedRoot -PathType Container)) {
        throw "Expected directory '$ExpectedRoot' is missing."
    }
    if (-not (Test-Path -LiteralPath $ActualRoot -PathType Container)) {
        throw "Actual directory '$ActualRoot' is missing."
    }
    $expectedFull = Get-TrimmedFullPath $ExpectedRoot
    $actualFull = Get-TrimmedFullPath $ActualRoot
    $expectedFiles = @(Get-ChildItem -LiteralPath $ExpectedRoot -Recurse -Force -File)
    $actualFiles = @(Get-ChildItem -LiteralPath $ActualRoot -Recurse -Force -File)
    if ($expectedFiles.Count -ne $actualFiles.Count) {
        throw "Directory file counts differ: expected $($expectedFiles.Count), found $($actualFiles.Count)."
    }
    $expected = @{}
    foreach ($file in $expectedFiles) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Directory '$ExpectedRoot' contains a reparse point, which is not allowed."
        }
        $relative = $file.FullName.Substring($expectedFull.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        $expected[$relative.ToLowerInvariant()] = $file.FullName
    }
    foreach ($file in $actualFiles) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Directory '$ActualRoot' contains a reparse point, which is not allowed."
        }
        $relative = $file.FullName.Substring($actualFull.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        $key = $relative.ToLowerInvariant()
        if (-not $expected.ContainsKey($key)) {
            throw "Unexpected file '$relative' in '$ActualRoot'."
        }
        $expectedHash = (Get-FileHash -LiteralPath $expected[$key] -Algorithm SHA256).Hash.ToLowerInvariant()
        $actualHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expectedHash -ne $actualHash) {
            throw "File '$relative' does not match its backup copy."
        }
    }
}

function Read-AndVerifyManifest {
    param(
        [string]$ManifestPath,
        [string]$StagingRoot
    )
    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
        throw "The staging manifest '$ManifestPath' is missing."
    }
    if (-not (Test-Path -LiteralPath $StagingRoot -PathType Container)) {
        throw "The staging directory '$StagingRoot' is missing."
    }

    $entries = @()
    $seen = @{}
    $lines = [IO.File]::ReadAllLines($ManifestPath, [Text.Encoding]::UTF8)
    foreach ($rawLine in $lines) {
        $line = $rawLine.Trim()
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        if ($line.Length -lt 67) {
            throw "The staging manifest contains a malformed line."
        }
        $hash = $line.Substring(0, 64)
        if ($hash -notmatch '^[0-9a-fA-F]{64}$') {
            throw "The staging manifest contains an invalid SHA-256 value."
        }
        if ($line.Substring(64, 2) -ne '  ') {
            throw "The staging manifest does not use the required '<hash>  <path>' format."
        }
        $relative = $line.Substring(66).Replace('\', '/')
        if ([string]::IsNullOrEmpty($relative) -or $relative.IndexOf([char]0) -ge 0) {
            throw "The staging manifest contains an empty or invalid path."
        }
        if ($relative.StartsWith('/') -or $relative.Contains(':')) {
            throw "The staging manifest path '$relative' is absolute or contains a drive separator."
        }
        $parts = $relative.Split([char]'/')
        foreach ($part in $parts) {
            if ([string]::IsNullOrEmpty($part) -or $part -eq '.' -or $part -eq '..') {
                throw "The staging manifest path '$relative' contains an unsafe component."
            }
            if ($part.EndsWith(' ') -or $part.EndsWith('.')) {
                throw "The staging manifest path '$relative' contains a name Windows would normalize."
            }
            if ($part.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0) {
                throw "The staging manifest path '$relative' contains invalid characters."
            }
        }
        $key = $relative.ToLowerInvariant()
        if ($seen.ContainsKey($key)) {
            throw "The staging manifest contains duplicate path '$relative'."
        }
        $seen[$key] = $true

        $nativeRelative = $relative.Replace('/', [IO.Path]::DirectorySeparatorChar)
        $fullPath = [IO.Path]::GetFullPath((Join-Path $StagingRoot $nativeRelative))
        if (-not (Test-PathWithin $fullPath $StagingRoot)) {
            throw "The staging manifest path '$relative' escapes the staging directory."
        }
        if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
            throw "The staged file '$relative' is missing."
        }
        $actual = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $hash.ToLowerInvariant()) {
            throw "The staged file '$relative' failed SHA-256 checksum verification."
        }
        $entries += [pscustomobject]@{
            Relative = $relative
            FullPath = $fullPath
            Hash = $actual
        }
    }

    if ($entries.Count -eq 0) {
        throw "The staging manifest is empty."
    }
    if (-not $seen.ContainsKey('akari-sweeps.exe')) {
        throw "The staged package does not contain 'akari-sweeps.exe'."
    }
    $hasSprite = $false
    foreach ($key in $seen.Keys) {
        if ($key -match '^assets/akari/.+\.png$') {
            $hasSprite = $true
            break
        }
    }
    if (-not $hasSprite) {
        throw "The staged package does not contain Akari sprite assets under 'assets/akari/'."
    }

    $stagingFull = Get-TrimmedFullPath $StagingRoot
    foreach ($file in @(Get-ChildItem -LiteralPath $StagingRoot -Recurse -Force -File)) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "The staging directory contains a reparse point, which is not allowed."
        }
        $relative = $file.FullName.Substring($stagingFull.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        if (-not $seen.ContainsKey($relative.ToLowerInvariant())) {
            throw "The staging directory contains unlisted file '$relative'."
        }
    }

    return $entries
}

function Wait-ForAkariProcesses {
    param(
        [string]$InstallDir,
        [int]$TimeoutSeconds
    )
    $timeout = [Math]::Max(0, $TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($timeout)
    $announced = $false

    while ($true) {
        $blocked = @()
        foreach ($name in $script:AliasNames) {
            $path = Join-Path $InstallDir $name
            if (Test-Path -LiteralPath $path -PathType Leaf) {
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
        }

        $running = @()
        try {
            $processNames = @($script:AliasNames | ForEach-Object { [IO.Path]::GetFileNameWithoutExtension($_) })
            $installFull = Get-TrimmedFullPath $InstallDir
            foreach ($process in @(Get-Process -Name $processNames -ErrorAction SilentlyContinue)) {
                $processPath = $null
                try {
                    $processPath = $process.Path
                } catch {
                    $processPath = $null
                }
                if (-not [string]::IsNullOrEmpty($processPath)) {
                    $processParent = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($processPath))
                    if ([string]::Equals((Get-TrimmedFullPath $processParent), $installFull, [StringComparison]::OrdinalIgnoreCase)) {
                        $running += ('{0} (PID {1})' -f [IO.Path]::GetFileName($processPath), $process.Id)
                    }
                }
            }
        } catch {
            # Process enumeration is best-effort; the exclusive file-open checks
            # above remain the authoritative lock test.
        }

        if ($blocked.Count -eq 0 -and $running.Count -eq 0) {
            return
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            $details = @()
            if ($blocked.Count -gt 0) {
                $details += ('locked files: ' + ($blocked -join ', '))
            }
            if ($running.Count -gt 0) {
                $details += ('running processes: ' + ($running -join ', '))
            }
            $detailText = ''
            if ($details.Count -gt 0) {
                $detailText = ' ' + ($details -join '; ')
            }
            $timeoutMessage = ("Timed out after $timeout seconds waiting for Akari-Sweeps to exit.$detailText " +
                "Close every Akari-Sweeps window from this installation and run 'akari update' again.")
            throw (New-Object System.TimeoutException -ArgumentList $timeoutMessage)
        }
        if (-not $announced) {
            Write-Log 'Waiting for running Akari-Sweeps processes to exit before replacing files...'
            $announced = $true
        }
        Start-Sleep -Milliseconds 750
    }
}

function Backup-Installation {
    param(
        [string]$InstallDir,
        [string]$BackupDir
    )
    $aliasBackup = Join-Path $BackupDir 'aliases'
    New-Item -ItemType Directory -Path $aliasBackup -Force | Out-Null

    foreach ($name in $script:AliasNames) {
        $source = Join-Path $InstallDir $name
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            $script:AliasesExisted[$name] = $true
            $destination = Join-Path $aliasBackup $name
            Copy-Item -LiteralPath $source -Destination $destination -Force
            $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
            $backupHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($sourceHash -ne $backupHash) {
                throw "Backup verification failed for '$name'."
            }
        } elseif (Test-Path -LiteralPath $source) {
            throw "Installation path '$source' is not a regular file; refusing to replace it."
        } else {
            $script:AliasesExisted[$name] = $false
            Write-Log "Alias '$name' is not present; the update will create it." 'WARN'
        }
    }

    $assets = Join-Path $InstallDir 'assets'
    if (Test-Path -LiteralPath $assets -PathType Container) {
        $script:AssetsExisted = $true
        $assetsBackup = Join-Path $BackupDir 'assets'
        if (Test-Path -LiteralPath $assetsBackup) {
            throw "Backup path '$assetsBackup' already exists."
        }
        Copy-Item -LiteralPath $assets -Destination $assetsBackup -Recurse -Force
        Assert-TreeMatches $assets $assetsBackup
        Write-Log 'Backed up the existing assets directory.'
    } elseif (Test-Path -LiteralPath $assets) {
        throw "Installation path '$assets' is not a directory."
    } else {
        $script:AssetsExisted = $false
        Write-Log "The assets directory is not present; the update will create it." 'WARN'
    }

    $state = New-Object System.Collections.Generic.List[string]
    foreach ($name in $script:AliasNames) {
        [void]$state.Add(('{0}={1}' -f $name, $script:AliasesExisted[$name]))
    }
    [void]$state.Add(('assets={0}' -f $script:AssetsExisted))
    $statePath = Join-Path $BackupDir 'state.txt'
    [IO.File]::WriteAllLines($statePath, [string[]]$state.ToArray(), (New-Object System.Text.UTF8Encoding($false)))
}

function Install-NewFiles {
    param(
        [string]$InstallDir,
        [string]$StagingDir,
        $Entries
    )
    $stagedExe = Join-Path $StagingDir 'akari-sweeps.exe'
    $header = [IO.File]::ReadAllBytes($stagedExe)
    if ($header.Length -lt 2 -or $header[0] -ne 0x4D -or $header[1] -ne 0x5A) {
        throw "The staged executable is not a valid Windows executable."
    }
    $canonicalHash = (Get-FileHash -LiteralPath $stagedExe -Algorithm SHA256).Hash.ToLowerInvariant()

    $script:ReplacementStarted = $true
    foreach ($name in $script:AliasNames) {
        $destination = Join-Path $InstallDir $name
        $temp = Join-Path $InstallDir ('.' + $name + '.new-' + [Guid]::NewGuid().ToString('N'))
        [void]$script:PendingTemps.Add($temp)
        Copy-Item -LiteralPath $stagedExe -Destination $temp -Force
        $tempHash = (Get-FileHash -LiteralPath $temp -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($tempHash -ne $canonicalHash) {
            throw "The staged copy of '$name' failed verification before replacement."
        }
        Move-Item -LiteralPath $temp -Destination $destination -Force
        [void]$script:PendingTemps.Remove($temp)
        Write-Log "Replaced $name"
    }

    if ($TestFailAfterAliasesReplace) {
        throw 'Test fault injection: simulated failure after replacing executable aliases.'
    }

    $installAssets = Join-Path $InstallDir 'assets'
    $stagedAssets = Join-Path $StagingDir 'assets'
    if (-not (Test-Path -LiteralPath $stagedAssets -PathType Container)) {
        throw "The staged package does not contain an assets directory."
    }
    $script:AssetsTouched = $true
    if (Test-Path -LiteralPath $installAssets) {
        Remove-Item -LiteralPath $installAssets -Recurse -Force
    }
    Copy-Item -LiteralPath $stagedAssets -Destination $installAssets -Recurse -Force
    Write-Log 'Replaced the assets directory.'
}

function Verify-Installation {
    param(
        [string]$InstallDir,
        [string]$StagingDir,
        $Entries
    )
    $stagedExe = Join-Path $StagingDir 'akari-sweeps.exe'
    $canonicalHash = (Get-FileHash -LiteralPath $stagedExe -Algorithm SHA256).Hash.ToLowerInvariant()
    foreach ($name in $script:AliasNames) {
        $path = Join-Path $InstallDir $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Executable '$name' is missing after replacement."
        }
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne $canonicalHash) {
            throw "Executable '$name' does not match the released executable after replacement."
        }
    }

    $installAssets = Join-Path $InstallDir 'assets'
    if (-not (Test-Path -LiteralPath $installAssets -PathType Container)) {
        throw "The assets directory is missing after replacement."
    }
    $installFull = Get-TrimmedFullPath $InstallDir
    $expectedAssets = @{}
    $spriteFound = $false
    foreach ($entry in $Entries) {
        if ($entry.Relative.StartsWith('assets/', [StringComparison]::OrdinalIgnoreCase)) {
            $expectedAssets[$entry.Relative.ToLowerInvariant()] = $entry
            if ($entry.Relative -match '^assets/akari/.+\.png$') {
                $spriteFound = $true
            }
        }
    }
    if ($expectedAssets.Count -eq 0) {
        throw "The release package contains no asset files."
    }
    if (-not $spriteFound) {
        throw "The release package contains no Akari sprite PNG files."
    }
    foreach ($entry in $Entries) {
        if (-not $entry.Relative.StartsWith('assets/', [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $nativeRelative = $entry.Relative.Replace('/', [IO.Path]::DirectorySeparatorChar)
        $path = [IO.Path]::GetFullPath((Join-Path $InstallDir $nativeRelative))
        if (-not (Test-PathWithin $path $InstallDir)) {
            throw "Installed asset path '$($entry.Relative)' escapes the installation directory."
        }
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Installed asset '$($entry.Relative)' is missing."
        }
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne $entry.Hash) {
            throw "Installed asset '$($entry.Relative)' does not match the release package."
        }
    }
    foreach ($file in @(Get-ChildItem -LiteralPath $installAssets -Recurse -Force -File)) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "The installed assets directory contains a reparse point, which is not allowed."
        }
        $relative = $file.FullName.Substring($installFull.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        if (-not $expectedAssets.ContainsKey($relative.ToLowerInvariant())) {
            throw "Unexpected file '$relative' appeared in the installed assets directory."
        }
    }
    Write-Log 'Verified all six executable aliases and the complete assets tree.' 'INFO'
}

function Restore-Backup {
    param(
        [string]$InstallDir,
        [string]$BackupDir
    )
    $problems = @()

    if ($script:AssetsTouched) {
        $installAssets = Join-Path $InstallDir 'assets'
        try {
            if (Test-Path -LiteralPath $installAssets) {
                Remove-Item -LiteralPath $installAssets -Recurse -Force -ErrorAction Stop
            }
        } catch {
            $problems += "could not remove the new assets directory ($($_.Exception.Message))"
        }
        if ($script:AssetsExisted) {
            try {
                $backupAssets = Join-Path $BackupDir 'assets'
                if (-not (Test-Path -LiteralPath $backupAssets -PathType Container)) {
                    throw "the assets backup is missing"
                }
                if (Test-Path -LiteralPath $installAssets) {
                    Remove-Item -LiteralPath $installAssets -Recurse -Force -ErrorAction Stop
                }
                Copy-Item -LiteralPath $backupAssets -Destination $installAssets -Recurse -Force -ErrorAction Stop
                Assert-TreeMatches $backupAssets $installAssets
            } catch {
                $problems += "could not restore assets ($($_.Exception.Message))"
            }
        }
    }

    $aliasBackup = Join-Path $BackupDir 'aliases'
    foreach ($name in $script:AliasNames) {
        $destination = Join-Path $InstallDir $name
        try {
            if (-not $script:AliasesExisted.ContainsKey($name)) {
                throw "the original state of '$name' was not recorded"
            }
            if ($script:AliasesExisted[$name]) {
                $backup = Join-Path $aliasBackup $name
                if (-not (Test-Path -LiteralPath $backup -PathType Leaf)) {
                    throw "the backup of '$name' is missing"
                }
                Copy-Item -LiteralPath $backup -Destination $destination -Force -ErrorAction Stop
                $backupHash = (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash.ToLowerInvariant()
                $restoredHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($backupHash -ne $restoredHash) {
                    throw "the restored file does not match its backup"
                }
            } elseif (Test-Path -LiteralPath $destination) {
                Remove-Item -LiteralPath $destination -Force -ErrorAction Stop
            }
        } catch {
            $problems += "could not restore '$name' ($($_.Exception.Message))"
        }
    }

    foreach ($temp in @($script:PendingTemps)) {
        try {
            if (Test-Path -LiteralPath $temp) {
                Remove-Item -LiteralPath $temp -Force -ErrorAction Stop
            }
        } catch {
            $problems += "could not remove temporary file '$temp' ($($_.Exception.Message))"
        }
    }

    if ($problems.Count -gt 0) {
        throw ($problems -join '; ')
    }
}

try {
    Write-Log 'Akari-Sweeps update helper started.'

    $script:InstallDir = Get-TrimmedFullPath $InstallDir
    $script:StagingDir = Get-TrimmedFullPath $StagingDir
    $script:ManifestPath = [IO.Path]::GetFullPath($ManifestPath)
    $script:BackupDir = Get-TrimmedFullPath $BackupDir

    if ((Test-PathWithin $script:StagingDir $script:InstallDir) -or
        (Test-PathWithin $script:InstallDir $script:StagingDir) -or
        (Test-PathWithin $script:BackupDir $script:InstallDir) -or
        (Test-PathWithin $script:InstallDir $script:BackupDir)) {
        throw "The update workspace and installation directory overlap; refusing to proceed."
    }
    $script:PathsValidated = $true

    if (-not (Test-Path -LiteralPath $script:InstallDir -PathType Container)) {
        throw "The installation directory '$script:InstallDir' does not exist."
    }
    if (-not (Test-Path -LiteralPath $script:StagingDir -PathType Container)) {
        throw "The staging directory '$script:StagingDir' does not exist."
    }

    Write-Log 'Verifying the staged release package before making any changes.'
    $entries = @(Read-AndVerifyManifest -ManifestPath $script:ManifestPath -StagingRoot $script:StagingDir)
    Write-Log "Staged package verified ($($entries.Count) files)."

    $probe = Join-Path $script:InstallDir ('.akari-write-test-' + [Guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::WriteAllText($probe, 'akari')
    } catch {
        throw ("Cannot write to the installation directory '$script:InstallDir'. " +
               "Run Akari-Sweeps from an account with permission to modify it, or reinstall to a writable location. " +
               "($($_.Exception.Message))")
    } finally {
        if (Test-Path -LiteralPath $probe) {
            Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue
        }
    }

    Wait-ForAkariProcesses -InstallDir $script:InstallDir -TimeoutSeconds $WaitSeconds
    Write-Log 'No running Akari-Sweeps processes are locking this installation.'

    Backup-Installation -InstallDir $script:InstallDir -BackupDir $script:BackupDir
    Write-Log 'Backup completed.'

    Install-NewFiles -InstallDir $script:InstallDir -StagingDir $script:StagingDir -Entries $entries
    Verify-Installation -InstallDir $script:InstallDir -StagingDir $script:StagingDir -Entries $entries

    $script:Committed = $true
    $script:ReplacementStarted = $false
    Write-Log 'Update completed successfully; all aliases and assets verified.' 'INFO'
    Remove-TransactionFiles
    exit 0
} catch {
    $exception = $_.Exception
    $message = $exception.Message
    Write-Log "Update failed: $message" 'ERROR'

    if ($script:Committed) {
        Write-Log 'The verified update is installed; temporary-file cleanup was incomplete.' 'WARN'
        exit 0
    }

    if ($script:ReplacementStarted) {
        try {
            Restore-Backup -InstallDir $script:InstallDir -BackupDir $script:BackupDir
            $script:ReplacementStarted = $false
            Write-Log 'Rollback completed; the previous installation was restored.' 'WARN'
            Remove-TransactionFiles
            exit 1
        } catch {
            $rollbackMessage = $_.Exception.Message
            Write-Log "ROLLBACK FAILED: $rollbackMessage" 'ERROR'
            Write-Log "Recovery files retained at: $script:BackupDir" 'ERROR'
            Write-Log "Close every Akari-Sweeps window, then restore the aliases and assets from that folder or reinstall with install.ps1." 'ERROR'
            exit 2
        }
    }

    Write-Log 'The existing installation was not modified.' 'WARN'
    Remove-TransactionFiles
    if ($exception -is [System.TimeoutException]) {
        exit 3
    }
    exit 1
}
