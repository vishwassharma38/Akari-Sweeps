# Akari-Sweeps

Windows maintenance TUI in Rust — a faithful port of `Akari-Sweeps.bat` (the behavioral spec) with Akari's idle reading, sweeping, health, about, and help-screen animations in a persistent terminal interface. The Rust app is the implementation of record; the BAT file remains as the auditable baseline, not the runtime engine.

**Contents**

[Purpose](#purpose) · [Architecture](#architecture) · [Commands](#commands) · [Administrator elevation](#administrator-elevation-uac) · [Sprite assets](#sprite-assets) · [Rendering + animation](#rendering--animation) · [Build](#build) · [Install](#install-regular-users-no-rust-required) · [Updates](#updates-github-releases) · [Publishing a release](#publishing-a-release) · [Errors & exit codes](#errors--exit-codes) · [Known limitations](#known-limitations) · [Tests](#tests)

---

## Purpose

- Reproduce the BAT's junk-cleanup targets (`[1/18]`–`[18/18]` directories, npm/pip, DNS/Recycle Bin, Disk Cleanup/SilentCleanup) and Windows servicing/repair commands with identical arguments and exit-code semantics, under strict cleaning/repair separation (see below).
- Present live output and per-operation results in a responsive Ratatui TUI while Akari animates in a dedicated right-side panel.
- Never claim success from "command launched" — only from measured results and real exit codes.

> **Note:** `DISM /StartComponentCleanup` historically ran as BAT `[18/18]` cleanup; it now runs as the first health step so `akari clean` launches zero DISM commands.

---

## Architecture

```
src/main.rs                 entry, exe-name dispatch, elevation startup, event loop
src/app.rs                  App state (screen, execution, phase, log, ops, input, animator)
src/commands/mod.rs         command registry, 5-item selection, input mapping,
                            phase identity (Phase) + capability gates (PhaseGate)
src/commands/clean.rs       cleanup plan ([1/18]–[18/18] dirs/programs, no DISM) + executor
src/commands/health.rs      health plan (DISM StartComponentCleanup + Check/Scan/Restore,
                            SFC, CHKDSK) + exit-code mapping
src/commands/all.rs         akari all sequencing (cleanup, then repair, merged summary)
src/commands/runner.rs      RealRunner + RecordingRunner (isolation tests)
src/system/mod.rs           OpStatus/OpResult/ExecEvent/MaintenanceRunner/AppSummary
src/system/elevation.rs     elevation policy, native runas relaunch, cancel handling
src/system/process.rs       run_program_capture(), program_exists()
src/system/windows.rs       TokenElevation check, free_space_kb()
src/akari/sprites.rs        animation-set discovery/validation/loading (RGBA output)
src/akari/animation.rs      Animator (idle/sweep/health/about/help sets;
                            Ambient/Sweeping/Settled), 6 FPS deadline
src/ui/*                    Ratatui layout: header, main/output, animation, entry
src/update/*                GitHub release discovery, checksum/archive validation,
                            staging, embedded PowerShell helper, update-check cache
install.ps1 / uninstall.ps1 developer build + copy-based command registration
install-release.ps1         standalone installer for published Windows releases
                            (no Rust/Cargo required)
.github/workflows/release.yml  tag-triggered Windows release pipeline
```

OS behavior stays behind `MaintenanceRunner`, so orchestration is unit-testable with mocks — no destructive ops run during `cargo test`.

---

## Commands

`akari` launches the interface. Inside it, exactly five commands are selectable (keys `1`–`5`):

| Key | Command        | Action                                             | Maintenance?   |
| :-: | -------------- | -------------------------------------------------- | -------------- |
| `1` | `akari clean`  | Cleanup only, zero DISM/SFC/CHKDSK                 | Cleanup only   |
| `2` | `akari health` | Health/servicing only (`[Repair 1/6]`–`[6/6]`)     | Repair only    |
| `3` | `akari all`    | Cleanup, then health, then combined summary        | Both, in order |
| `4` | `akari about`  | About (name, Cargo version, purpose, repo/license) | No             |
| `5` | `akari help`   | Usage, per-command help, exit instructions         | No             |

The same subcommands can be run directly from PowerShell, for example `akari clean`, `akari health`, `akari all`, `akari about`, and `akari help`. The existing concatenated executables (`akariclean`, `akarihealth`, `akariall`, `akariabout`, and `akarihelp`) remain supported for compatibility.

### Update commands (CLI, outside the TUI)

```powershell
akari --version        # Akari-Sweeps 0.1.0 (version comes from Cargo.toml)
akari update --check   # query GitHub, compare versions, download nothing
akari update           # download, verify, and install the latest release
```

Only the primary `akari` launcher accepts these updater options. Maintenance subcommands (`akari clean`, `akari health`, `akari all`) never make update network requests and keep their existing argument behavior.

### Command-entry field

Every screen shows a `Command` box. Press `/` (or Enter) to type, then Enter to run. Input is trimmed and case-insensitive; number selection and text entry share one dispatch path. Empty input does nothing; unknown names get a hint listing the valid commands. While typing, shortcuts suspend except Enter (run), Esc (cancel), Backspace (edit), and Ctrl+U (clear).

**Other keys**

| Key         | Action                                         |
| ----------- | ---------------------------------------------- |
| `t`         | all                                            |
| `h`         | home                                           |
| `↑` / `↓`   | scroll                                         |
| `q` / `Esc` | quit                                           |
| `Ctrl+C`    | deliberate cancel/quit (double-press while running) |

### Cleanup/repair isolation (guaranteed, strict)

- `akari clean` runs only cleanup. It invokes zero DISM commands of any kind (including `/StartComponentCleanup`), zero SFC, zero CHKDSK. `[18/18]` is Disk Cleanup + SilentCleanup only.
- `akari health` runs only the six health/servicing commands (DISM StartComponentCleanup, Check/Scan/RestoreHealth, SFC, CHKDSK). It deletes no files and invokes no cleanup.
- `akari all` runs cleanup to completion, then repair — never concurrent, never a second cleanup. **Failure policy:** cleanup errors never block repair, but they poison the combined summary, so overall status/exit code can never report success when a required phase failed.
- Enforcement is structural: phase runners require capability tokens (`CleanupGate`/`RepairGate`) minted only for permitted commands, plus invocation-log regression tests asserting the actual command order.

---

## Administrator elevation (UAC)

Windows elevation is detected from the current process access token using `OpenProcessToken` and `GetTokenInformation(TokenElevation)`. A successful negative result means restricted mode; an API error remains an unknown state and maintenance fails closed until a relaunched child verifies its token.

- The plain `akari` launcher requests standard Windows UAC elevation before terminal initialization. If approved, the same executable starts with the original arguments and an internal `ELEVATED` marker. The original process waits for that child and propagates its exit code, so only the elevated process owns the TUI. If the prompt is declined or the launch fails, the original process continues in restricted mode.
- Restricted mode keeps Home, About, and Help usable and displays a clear warning. Selecting maintenance requests UAC again in a background thread; the TUI stays navigable, and cancellation or another launch failure starts no maintenance and does not mark an operation as running.
- Direct maintenance entry points (`akari clean`, `akari health`, `akari all`, and the legacy executable aliases) request elevation before terminal or maintenance initialization. UAC cancellation returns `1223`; other shell, executable, or process-wait failures return an error with their diagnostic. Neither path runs cleanup, DISM, SFC, or CHKDSK in the parent.
- `akari about`, `akari help`, and informational TUI commands remain available without elevation. `akari --version` and updater CLI commands retain their early handling and do not trigger a UAC prompt.
- The `ELEVATED` marker is appended once and is not treated as a public command. A marked child verifies its actual token and aborts if Windows did not grant elevation, preventing relaunch loops.
- The launcher uses `ShellExecuteExW` with the `runas` verb, passes the current executable, safely quotes Windows arguments, and preserves the current working directory. It waits on the returned process handle and propagates the child's exit code. Only Windows `ERROR_CANCELLED` (`1223`) is reported as a declined prompt; other errors remain launch failures.
- A final token check occurs immediately before any maintenance runner is constructed and again at the worker boundary. Process creation failures retain a structured reason, and DISM, SFC, and CHKDSK report access-denied and elevation-required results separately from missing tools and unrelated failures.

### Manual Windows verification

UAC cannot be automated in CI. Use a disposable Windows 10/11 VM for servicing integration tests. Do not run the destructive full workflow against a primary personal installation.

| Step | Scenario | Verification |
| :-: | --- | --- |
| **A** | Normal interactive startup | Launch `akari` from a non-admin terminal, approve UAC, verify the elevated TUI opens and only one TUI is active. |
| **B** | Declined interactive startup | Launch `akari`, decline UAC, verify the restricted warning and that Home, About, and Help remain usable with no maintenance started. |
| **C** | Declined in-TUI maintenance | Request maintenance in restricted mode, decline UAC, verify the request clears, no worker or operation is marked running, and navigation remains available. |
| **D** | Direct maintenance startup | Launch `akari clean`, `akari health`, and `akari all` from a non-admin terminal, approve each UAC prompt, and verify work starts only in the elevated child. |
| **E** | Direct maintenance cancellation | Launch `akari all`, decline UAC, and verify neither phase runs, a cancellation message appears, and the process returns `1223`. |
| **F** | Already elevated | Launch `akari` and each maintenance alias from an administrator terminal; verify that no second UAC prompt appears. |
| **G** | Permission diagnostics | In a controlled VM, verify DISM, SFC, and CHKDSK permission failures remain failed results and identify access denial or required elevation while retaining useful command output. |
| **H** | Full `akari all` workflow | In a disposable VM, verify cleanup completes before repair, the four DISM operations and then SFC/CHKDSK run elevated, and the final summary and exit code match the real outcomes. |

Automated policy, quoting, and permission tests use mocks and do not display UAC dialogs or invoke servicing tools. They are not substitutes for these manual Windows checks.

---

## Sprite assets

Directories: `assets/akari/sweep/`, `assets/akari/idle/`, `assets/akari/health/`, `assets/akari/about/`, and `assets/akari/help/` (shipped to `<InstallDir>\assets\…`).

Each directory contains numbered PNGs using its matching prefix, such as `sweep_001.png`, `idle_001.png`, `health_001.png`, `about_001.png`, and `help_001.png`. Every valid numbered PNG in a set is loaded automatically.

### Frame discovery

Frame filenames are discovered at runtime, not limited to a hardcoded count: every `<prefix>_<digits>.png` file in a set's directory is loaded and sorted by its numeric index, so `_009`, `_010`, and beyond continue the loop naturally. Leading zeros are optional (`help_1.png` and `help_001.png` are the same index). Non-PNG files (such as this project's README files) are ignored. Numeric gaps, duplicate numeric indices, and PNG files that do not match the naming convention produce an explicit diagnostic in the animation panel rather than ambiguous playback order.

Every discovered file is kept as its own frame; identical bytes are never deduplicated or skipped. The About and Help folders include byte-identical frames in the current artwork, and those files retain separate playback positions. Playback counts come from the frames that actually loaded, so adding, removing, or extending a sequence needs no source-code change.

### Image handling

PNG frames may use any dimensions and aspect ratio. Frames within one animation may even differ in size; no resizing or source-code change is required. RGB and RGBA PNGs are accepted, converted to RGBA internally, and alpha transparency is preserved (opaque artwork keeps its own background).

At startup each set's frames are decoded in numeric order and measured automatically. The loader builds one shared transparent canvas sized to the set's largest width and height, then centers every original image on it without stretching, cropping, or rewriting the source PNGs. Missing, corrupt, or unsupported frames produce an explicit diagnostic in the animation panel — the TUI stays usable. A resource-safety budget catches absurd image sizes with a clear message instead of exhausting memory. A set with no usable frames shows the diagnostic instead of animating, and the rest of the TUI remains usable.

### Animation sets by screen

| Screen / activity                                                 | Frames played             |
| ----------------------------------------------------------------- | ------------------------- |
| Home (idle mode)                                                  | Book-reading frames       |
| About                                                             | Magnifying-glass frames   |
| Help                                                              | Its own copy of the magnifying-glass frames |
| Cleanup                                                           | Sweep frames              |
| Health/repair work (including the health phase of `akari all`)    | Health frames             |

### Replacing an animation

1. Open its directory under `assets/akari/`.
2. Replace the PNGs, or add more frames with the next numeric indices.
3. Keep the `<prefix>_NNN.png` naming and a contiguous 1-based sequence (for example `help_001.png`, `help_002.png`, …). Extra frames play automatically.
4. Build and run the application.

### Additional animation sets

Help is registered as its own set and selected on the Help screen. Future sets follow the same path: add a frame-set spec, extend `akari::animation::AnimationSet`, and select it from `App::sync_animation`; the TUI panel needs no rewrite.

---

## Rendering + animation

- Each half-block cell half is an **area average** over its source pixels (box filter): gradients survive downscaling instead of aliasing, and alpha composites over the terminal background during averaging.
- Every frame is rendered through its animation set's shared virtual canvas, so one scale/coordinate system is used for the whole loop even when replacement frames have different dimensions.
- Aspect preserved, centered on both axes (no per-frame drift), with transparent padding kept transparent (no black rectangles).
- True-color RGB output; terminals without true color degrade gracefully to shape-only rendering (no crash, no claim of pixel-exact output).
- **6 FPS** (≈167 ms/frame) for all animations via deadline scheduling on a monotonic clock: no cumulative drift, stalls resync instead of queuing backlog, redraws only on change, input polled within ≤50 ms — no busy loop, keyboard never blocked.

---

## Build

```powershell
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --release --locked   # -> target\release\akari-sweeps.exe
```

---

## Install (regular users, no Rust required)

Supported platform: **Windows 10/11 on x86_64**. Rust, Cargo, and a source checkout are not required. The installer installs the prebuilt application and adds its folder to the **current user's** PATH; administrator privileges are not required for installation.

### One-command install (PowerShell)

After the first stable GitHub Release has been published, open PowerShell and paste this one-line command:

```powershell
& { $ErrorActionPreference='Stop'; $p=Join-Path $env:TEMP ('akari-sweeps-'+[guid]::NewGuid().ToString('N')+'.ps1'); try { Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1' -OutFile $p; & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $p; if ($LASTEXITCODE -ne 0) { throw "Akari-Sweeps installer failed with exit code $LASTEXITCODE." } } finally { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue } }
```

This downloads the installer from the latest stable GitHub Release, runs it in a separate Windows PowerShell process, and removes the temporary script afterward. The installer downloads the matching application ZIP and SHA-256 sidecar, verifies the checksum, validates the archive, installs all six command aliases and the complete assets tree, then updates the current user's PATH. The terminal you pasted into stays open.

The default install directory is `%LOCALAPPDATA%\Akari-Sweeps`. Restart the terminal after the first installation, then verify:

```powershell
akari --version
akari help
```

### Install from the itch.io page

The release workflow attaches `akari-sweeps-itchio-setup.zip` to every published stable GitHub Release. It can also publish that same validated bundle to your itch.io project's persistent Windows download channel using Butler. Configure the one-time GitHub Actions secret and repository variable described in `ITCHIO_SETUP.md`; after that, each stable release updates the same itch.io download slot automatically.

Keep these permanent fallback links in the itch.io page description:

- **Setup bundle:**
  <https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/akari-sweeps-itchio-setup.zip>
- **One-command installer:**
  <https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1>

GitHub resolves `/releases/latest/download/<asset-name>` to the matching asset on the latest stable release. Because the asset filenames stay the same, these links do not need to be edited for later versions. The one-command PowerShell installation above downloads the current installer, which then fetches and verifies the matching application package.

For the locally downloaded bundle, extract all files into one folder, open PowerShell there, and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install-release.ps1 -PackagePath .\akari-sweeps-windows-x86_64.zip
```

The installer automatically uses the adjacent `akari-sweeps-windows-x86_64.zip.sha256` sidecar. It verifies and installs the local package without contacting GitHub or downloading the application again. Keep the installer, ZIP, and checksum sidecar together.

### Direct installer options

- Download the installer separately: <https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1>
- `-Version` accepts `0.1.1` or `v0.1.1`; omit it to install the latest non-prerelease GitHub release.
- `-PackagePath` installs from a local release ZIP and performs no network download. Its checksum defaults to the package path plus `.sha256`.
- `-ChecksumPath` optionally selects a different SHA-256 sidecar.
- `-InstallDir` selects a custom installation directory.

Existing unrelated files in the install directory are preserved. The six application aliases and the application-owned assets directory are replaced; re-running the installer is safe. PATH updates are idempotent: unrelated entries are preserved, duplicates are not created, and the machine-wide PATH is never modified.

### Uninstall / undo

`uninstall.ps1` lives in the repository (it is not required for installation and is not part of the release ZIP). Run it from a checkout or download it next to your shell. It removes only the six aliases, `assets\`, and the matching user-PATH entry; unrelated files are preserved.

```powershell
.\uninstall.ps1                    # removes exes, assets, and the PATH entry
.\uninstall.ps1 -InstallDir "D:\Tools\Akari"
```

---

## Install (developers)

Chosen strategy: **one binary copied under six names** + exe-stem dispatch. No permanent BAT wrapper; each alias is native (CWD-independent, args forwarded, exit codes native).

```powershell
.\install.ps1                      # build + install to %LOCALAPPDATA%\Akari-Sweeps
.\install.ps1 -InstallDir "D:\Tools\Akari"   # custom location
.\install.ps1 -NoBuild             # reuse target\release\akari-sweeps.exe
```

Adds the install dir to the **user** PATH (machine PATH untouched). Restart the terminal, then `akari help` should open the TUI. This developer path still requires Rust/Cargo; normal users should use `install-release.ps1` instead.

---

## Updates (GitHub Releases)

Akari-Sweeps uses the public GitHub Releases API for `vishwassharma38/Akari-Sweeps`. No token, account, separate update server, or installed updater executable is required.

### Update behavior

- `akari update --check` prints the installed version, latest published version, status, and release page. It downloads and changes nothing.
- `akari update` downloads only the exact assets `akari-sweeps-windows-x86_64.zip` and `akari-sweeps-windows-x86_64.zip.sha256`, verifies SHA-256, validates the archive, and stages it outside the installation directory.
- The package's single `akari-sweeps.exe` replaces **all six** executable aliases. The installed `assets/` tree is completely refreshed, so removed sprites do not linger. Unrelated files (configuration, user data, other directories) are not touched.
- The installation directory is derived from the running executable, not from a hardcoded default. Custom layouts created with `.\install.ps1 -InstallDir "D:\Tools\Akari"` update in place, and another Akari-Sweeps installation on the same machine is never modified.
- Windows cannot replace a running executable. The Rust process launches an embedded PowerShell 5.1 helper and exits; the helper waits for all Akari-Sweeps processes in the same installation to exit, backs up the old aliases/assets, replaces them, verifies the result, and rolls back every change if any step fails. It never terminates Akari-Sweeps processes. If files remain locked until the timeout, nothing is replaced and the helper explains which processes to close.
- Progress and errors are appended to `%LOCALAPPDATA%\Akari-Sweeps\update.log`. If rollback itself fails, the backup directory is retained and its location is reported explicitly.

### Automatic notification

Launching the primary `akari` interface starts a background GitHub check. The TUI appears immediately and never waits for the network.

The check state is tracked in `%LOCALAPPDATA%\Akari-Sweeps\update-check.json` (independent of the installation directory):

- A **successful** release API response - including "already up to date" - suppresses the next automatic check for 24 hours.
- A failed check (no published release yet, DNS/connection/timeout, HTTP 5xx, invalid response, or rate limiting) never creates the 24-hour success cache. It is retried after **1 hour**, so a release published shortly after a 404 is discovered promptly. A numeric `Retry-After` rate-limit value is honored within bounded limits (1 minute to 6 hours).
- Old cache files remain readable, and a missing, corrupt, or unwritable cache behaves like a missing cache; it can never block startup or the TUI.
- Manual `akari update` / `akari update --check` bypasses the cache entirely and always works, even immediately after a failed automatic check.

When a newer release exists, the Home screen shows a yellow update banner with the release page and the Output log gets a matching entry.

Automatic checks **only notify**. They never download, install, or replace anything; the user must explicitly run `akari update`. Maintenance commands (`akari clean`, `akari health`, `akari all`) do not perform automatic checks, and `akari about` / `akari help` do not either.

### Security notes

- Release asset URLs are parsed and compared component-by-component: HTTPS, host exactly `github.com`, owner/repository exactly `vishwassharma38/Akari-Sweeps`, path `/releases/download/<tag>/<expected-asset>`, no credentials, no unexpected port, and no lookalike host or unrelated repository. Arbitrary external HTTPS URLs are rejected.
- Redirects are followed manually, never automatically. Every destination is revalidated before it is requested; only the same GitHub release-asset path and GitHub's signed release-asset CDN hosts are accepted. HTTP downgrades, external hosts, credentials, unexpected ports, and chains longer than five redirects are rejected. Signed CDN query parameters are preserved.
- The compressed ZIP download is limited to **100 MiB**. A declared `Content-Length` over the limit is rejected before writing, and the actual streamed byte count is enforced independently so missing or inaccurate headers cannot bypass the limit. Rejected or interrupted downloads are deleted and never reach extraction or installation.
- The SHA-256 sidecar is checked against the exact downloaded ZIP before any extraction. If GitHub also supplies a `sha256:` asset digest, it must agree with both.
- ZIP entries with absolute paths, `..`, drive/alternate-data-stream separators, symlinks, duplicate names, unexpected executables outside `assets/`, or contents other than the expected executable/assets tree are rejected. Extraction uses an isolated staging directory and explicit size limits.
- Before changing the installation, the PowerShell helper re-verifies a Rust-generated SHA-256 manifest of every staged file.
- The standalone installer verifies the published SHA-256 sidecar before extracting anything, checks the expected executable and all five sprite sets, and never executes files from the archive while installing.
- SHA-256 detects accidental or inconsistent archive changes. A checksum and archive published through the same GitHub account do not provide independent publisher authentication. Authenticode/release signing is a possible future enhancement, not part of this implementation.

---

## Publishing a release

`.github/workflows/release.yml` runs when a `v*` tag is pushed. It:

1. Checks out the repository and installs stable Rust for `x86_64-pc-windows-msvc`.
2. Reads the `[package]` version from `Cargo.toml` and fails unless the tag is exactly `v` + that version.
3. Runs `cargo test --locked`.
4. Builds the optimized executable.
5. Validates `install-release.ps1` and runs its standalone installer tests.
6. Packages `akari-sweeps.exe` and the complete `assets/` tree as `akari-sweeps-windows-x86_64.zip`.
7. Verifies the ZIP contains the expected executable and sprite, then writes `akari-sweeps-windows-x86_64.zip.sha256` as UTF-8 without a BOM.
8. Builds and validates `akari-sweeps-itchio-setup.zip`, a self-contained setup bundle containing the Windows package, its checksum sidecar, `install-release.ps1`, and `INSTALL.txt`. The bundle is attached to GitHub Releases and downloaded through a permanent latest-release link on itch.io.
9. Publishes the Windows ZIP, checksum sidecar, standalone installer, and setup bundle with GitHub-generated release notes using the built-in `GITHUB_TOKEN` (`contents: write` at job level).
10. After the GitHub Release is published, the optional `publish-itchio` job downloads and validates the exact setup bundle, then pushes it to the configured itch.io Windows channel with Butler. The job runs when the `ITCHIO_TARGET` repository variable is configured.

`Cargo.toml` is the only version source. The About screen, `akari --version`, and the updater all derive from it.

### Release steps

```powershell
# 1. Bump the [package] version in Cargo.toml (e.g. 0.1.0 -> 0.1.1).

# 2. Refresh Cargo.lock with a normal (unlocked) Cargo command so the
#    akari-sweeps package version in the lockfile matches Cargo.toml.
#    `cargo check --locked` here would fail on the stale lockfile.
cargo check

# 3. Inspect the lockfile change: the package version must be updated and no
#    dependency versions should change unintentionally.
git diff -- Cargo.toml Cargo.lock

# 4. Verify the exact lockfile that will be committed.
cargo fmt --check
cargo test --locked
cargo check --locked
cargo build --release --locked

# 5. Commit BOTH the manifest and the refreshed lockfile before tagging.
git add Cargo.toml Cargo.lock
git commit -m "Release v0.1.1"
git push origin main

# 6. Create and push the matching tag. The tag must be exactly "v" followed
#    by the Cargo.toml version; the workflow rejects a mismatch.
git tag v0.1.1
git push origin v0.1.1
```

Pushing the tag starts the workflow; the GitHub Release exists only after the workflow completes successfully. There is no release if the tag and Cargo version differ, the lockfile is stale, tests fail, or the build/package step fails. Verify the result on the repository's **Actions** and **Releases** pages, including that the Windows ZIP, `.sha256` sidecar, `install-release.ps1`, and `akari-sweeps-itchio-setup.zip` are attached.

### itch.io connection

Configure the one-time itch.io connection by following `ITCHIO_SETUP.md`:

- Add the itch.io Butler API key as the GitHub Actions repository secret `BUTLER_API_KEY`.
- Add the project target as the GitHub Actions repository variable `ITCHIO_TARGET`, using the format `itch-username/project-slug:windows`.

After that one-time setup, every new stable GitHub Release publishes the same setup bundle to the same itch.io Windows channel. You do not need to change the itch.io page URL, re-upload a file, or edit the secret/variable for each release. Keep the stable GitHub latest-asset links in the page description as a fallback.

The `publish-itchio` job is skipped when `ITCHIO_TARGET` is not configured. If the target is configured but `BUTLER_API_KEY` is missing or invalid, the publishing job fails and reports the missing/invalid credential rather than silently claiming that itch.io was updated.

---

## Errors & exit codes

Summary rule mirrors BAT `:ShowSummary`:

| Outcome                      | Exit code |
| ---------------------------- | :-------: |
| Errors + some success        |    `1`    |
| Errors + zero success        |    `2`    |
| Clean                        |    `0`    |

Warnings (e.g. undocumented `cleanmgr` codes, unverifiable Recycle Bin result) show as `!` and never affect the exit code. DISM `3010` = success + "restart required" warning. CHKDSK `0/1/2/3+` mapped per BAT. Locked files are skipped + counted (`files_left`), unmeasurable folders flagged.

---

## Known limitations

- Repair needs Windows 10 servicing stack + elevation; on non-Windows hosts the TUI runs but Windows-only subprocesses fail honestly, and elevation is skipped (dev/test path) with a notice.
- UAC interaction itself can't be covered by automated tests (manual procedure above); the decision/relaunch/quote logic is unit-tested.
- Disk-space deltas include concurrent drive activity (same caveat as BAT).
- Animation is a scaled approximation, not pixel-exact blitting.
- `akari all` runs repair without a second Y/N gate — the explicit alias is treated as consent (documented deviation from the BAT's prompt).

---

## Tests

```powershell
cargo test --locked
powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\install-release.tests.ps1
```

Coverage includes dispatch, input/back navigation, placeholder behavior, elevation decisions, strict cleanup/repair isolation, sprites and animation selection, rendering, exit codes, and the updater: semver comparison, release metadata/asset selection, exact download-URL and redirect validation, compressed-download limits (missing/inaccurate `Content-Length` and streamed overruns), checksum and API-digest verification, hostile ZIP rejection, success/short-retry cache behavior, dynamic installation discovery, safe helper arguments, an installed-copy fixture that stages a release ZIP and updates a temporary installation, injected rollback, and a locked executable timing out without changing files.

The standalone installer tests run offline on Windows PowerShell 5.1 with isolated temporary directories and a fake per-user PATH. They cover remote and local package installation, explicit and inferred checksum paths, checksum mismatch, missing package/downloads/checksum, missing executable or assets, extraction failure, an existing install directory, repeated installs, PATH preservation and duplicate prevention, unsafe directories, invalid versions and argument combinations, cleanup after failure, and the absence of any Cargo/Rust invocation. The real user's PATH and any real Akari-Sweeps installation are never touched.

Destructive/real servicing ops never run in tests, and updater tests never touch the real installation or the live GitHub API. TUI resize/exit-code cases are covered by `TestBackend` layout tests; UAC prompts, live maintenance runs, the real PowerShell replacement of an installed build, the standalone installer downloading an actual published release, and the GitHub Actions release itself remain manual/infrastructure verification.

### Manual release-to-update verification

Perform this only after the first baseline release is published and use a disposable Windows test installation, never a real user installation.

1. Publish `v0.1.0`; confirm the release contains the Windows ZIP, `.sha256` sidecar, `install-release.ps1`, and `akari-sweeps-itchio-setup.zip`, and that GitHub Actions succeeded.
2. Test both installation routes on a disposable Windows x86_64 machine: run the one-command PowerShell install, then separately extract the itch.io setup bundle and install with `-PackagePath`. In a new terminal confirm `akari --version`, all six command names, sprites, and PATH.
3. Prepare and publish `v0.1.1` with the corrected Cargo.lock workflow above.
4. From the disposable `0.1.0` installation run `akari update --check` and then `akari update`. The command exits after launching the separate helper; inspect `%LOCALAPPDATA%\Akari-Sweeps\update.log` and wait for completion.
5. Confirm `akari --version` reports `Akari-Sweeps 0.1.1`, all six aliases and all assets were replaced, and unrelated files remain.
6. Repeat with a deliberately failed replacement and confirm the `0.1.0` files are restored and the failure is explained in the log.