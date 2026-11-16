# Akari-Sweeps

**A Windows terminal maintenance toolkit with an animated Akari interface.**

Akari-Sweeps brings Windows cleanup and system-health tools into a persistent terminal UI, with live operation output, clear results, and Akari animations.

## Features

- **Animated terminal interface:** A responsive Ratatui TUI with idle, sweeping, health, About, and Help animations.
- **Six terminal commands:** Use the main akari interface or launch a specific maintenance/info command directly.
- **Cleanup and health are separated:** Run cleanup independently, health/repair independently, or both in sequence.
- **Measured results:** Operations report real process results and exit codes rather than treating a launched command as a success.
- **Built-in update support:** Check for and install published updates using akari update.
- **No developer toolchain required:** The release installer uses the prebuilt Windows application. Rust and Cargo are not needed.

## Quick install from PowerShell

**Requirements:** Windows 10 or Windows 11, 64-bit (x86_64).

Open PowerShell and paste this single command:

~~~powershell
& { $ErrorActionPreference='Stop'; $p=Join-Path $env:TEMP ('akari-sweeps-'+[guid]::NewGuid().ToString('N')+'.ps1'); try { Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1' -OutFile $p; & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $p; if ($LASTEXITCODE -ne 0) { throw "Akari-Sweeps installer failed with exit code $LASTEXITCODE." } } finally { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue } }
~~~

This downloads the official installer from the latest stable GitHub Release. The installer then downloads the matching Windows package, verifies its published SHA-256 checksum, validates the archive, installs the application, and adds its folder to your **current user's PATH**. The installation itself does not require administrator privileges.

The default installation folder is:

~~~text
%LOCALAPPDATA%\Akari-Sweeps
~~~

Restart PowerShell or open a new terminal after installation.

## Download and install directly from itch.io

Prefer downloading a file from itch.io? Download **Akari-Sweeps Windows Setup Bundle** from the Download section and extract it.

Open PowerShell in the extracted folder and run:

~~~powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install-release.ps1 -PackagePath .\akari-sweeps-windows-x86_64.zip
~~~

Keep the installer, ZIP package, and .sha256 file together. This local installation route does not need to download the application again or contact GitHub. It verifies the package against the included checksum before installing.

## Available commands

| Command | What it does |
| --- | --- |
| akari | Open the interactive interface |
| akariclean | Run cleanup operations only |
| akarihealth | Run Windows health and repair operations only |
| akariall | Run cleanup, then health/repair operations, in sequence |
| akariabout | Show application information |
| akarihelp | Show usage and command help |

You can also enter the main interface and choose an action there.

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
- **Releases:** https://github.com/vishwassharma38/Akari-Sweeps/releases
- **Report an issue:** https://github.com/vishwassharma38/Akari-Sweeps/issues
