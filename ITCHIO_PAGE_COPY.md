# Akari-Sweeps

**A Windows terminal maintenance toolkit with an animated Akari interface.**

Akari-Sweeps brings Windows cleanup and system-health tools into a persistent terminal UI, with live operation output, clear results, and Akari animations.

## Download the latest Windows setup

**[Download Akari-Sweeps for Windows (latest stable release)](https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/akari-sweeps-itchio-setup.zip)**

This link always resolves to the setup bundle attached to the latest stable GitHub Release. You do not need to change this link when a new version is published.

The bundle includes the installer, application package, SHA-256 checksum, and installation guide. Download it, extract the ZIP, then run the local installation command below.

## One-command installation

**Requirements:** Windows 10 or Windows 11, 64-bit (x86_64).

Prefer not to download and extract a bundle manually? Open PowerShell and paste:

~~~powershell
& { $ErrorActionPreference='Stop'; $p=Join-Path $env:TEMP ('akari-sweeps-'+[guid]::NewGuid().ToString('N')+'.ps1'); try { Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1' -OutFile $p; & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $p; if ($LASTEXITCODE -ne 0) { throw "Akari-Sweeps installer failed with exit code $LASTEXITCODE." } } finally { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue } }
~~~

This downloads the official installer from the latest stable GitHub Release. The installer downloads the matching application package, verifies its published SHA-256 checksum, validates the archive, installs all six command aliases and their animation assets, and adds the installation folder to your **current user's PATH**.

Installation does not require Rust, Cargo, or administrator privileges. Restart your terminal after installation.

## Install from the downloaded setup bundle

1. Download the latest setup bundle using the link above.
2. Extract all files into one folder.
3. Open PowerShell in that folder and run:

~~~powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install-release.ps1 -PackagePath .\akari-sweeps-windows-x86_64.zip
~~~

The installer automatically reads the adjacent SHA-256 sidecar. It verifies the local package before installation and does not need to download the application again.

The default installation folder is %LOCALAPPDATA%\Akari-Sweeps.

## Features

- **Animated terminal interface:** A responsive Ratatui TUI with idle, sweeping, health, About, and Help animations.
- **Six terminal commands:** Use the main akari interface or launch a specific maintenance/info command directly.
- **Cleanup and health are separated:** Run cleanup independently, health/repair independently, or both in sequence.
- **Measured results:** Operations report real process results and exit codes rather than treating a launched command as a success.
- **Built-in update support:** Check for and install published updates using akari update.
- **No developer toolchain required:** The release installer uses the prebuilt Windows application.

## Available commands

| Command | What it does |
| --- | --- |
| akari | Open the interactive interface |
| akari clean | Run cleanup operations only |
| akari health | Run Windows health and repair operations only |
| akari all | Run cleanup, then health/repair operations, in sequence |
| akari about | Show application information |
| akari help | Show usage and command help |

The legacy concatenated executables remain supported for existing scripts.

## Updates

To check for updates:

~~~powershell
akari update --check
~~~

To download, verify, and install the latest stable release:

~~~powershell
akari update
~~~

The application only notifies about updates automatically. It does not silently download or install them.

## Important notes

- **Windows 10/11, 64-bit only.** This release is for x86_64 Windows systems.
- Installation adds commands to the current user's PATH. Open a new terminal after installation.
- Maintenance actions can modify system state and remove selected temporary/junk files. Review the operation and save your work before running maintenance.
- Windows may show a UAC prompt when an operation requires administrator privileges. Informational screens do not require elevation.
- Close Akari-Sweeps before reinstalling or updating.

## Source code and support

- **Source code:** https://github.com/vishwassharma38/Akari-Sweeps
- **Latest release notes:** https://github.com/vishwassharma38/Akari-Sweeps/releases/latest
- **Report an issue:** https://github.com/vishwassharma38/Akari-Sweeps/issues
