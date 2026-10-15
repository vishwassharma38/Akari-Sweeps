# Akari-Sweeps_V00.1

Windows maintenance TUI in Rust — a faithful port of `Akari-Sweeps.bat`
(the behavioral spec) with Akari's sweeping animation in a persistent
terminal interface. The Rust app is the implementation of record; the BAT
file remains as the auditable baseline, not the runtime engine.

## Purpose

- Reproduce the BAT's junk-cleanup phase ([1/18]–[18/18]) and optional
  Windows image-repair phase ([Repair 1/5]–[5/5]) with identical ordering,
  targets, arguments, and exit-code semantics.
- Present live output + per-operation results in a responsive Ratatui TUI
  while Akari sweeps in a dedicated right-side panel.
- Never claim success from "command launched" — only from measured results
  and real exit codes.

## Architecture

```
src/main.rs          entry, exe-name dispatch, terminal lifecycle, event loop
src/app.rs           App state (screen, execution, log, ops, animator, summary)
src/commands/mod.rs  6-command registry + resolve_command()
src/commands/clean.rs cleanup plan ([1/18]–[18/18]) + BAT-faithful executor
src/commands/health.rs repair plan (DISM×3, SFC, CHKDSK) + exit-code mapping
src/commands/all.rs  akariall sequencing (cleanup, then repair, merged summary)
src/commands/runner.rs RealRunner (filesystem + subprocess side effects)
src/system/mod.rs    OpStatus/OpResult/ExecEvent/MaintenanceRunner/AppSummary
src/system/process.rs run_program_capture(), program_exists()
src/system/windows.rs is_elevated() (fltmc, like BAT), free_space_kb()
src/akari/sprites.rs sweep discovery/validation/loading (256×320 RGBA)
src/akari/animation.rs Animator (Ambient/Sweeping/Settled) + future sets
src/ui/*             Ratatui layout: header, main/output, animation, status
install.ps1 / uninstall.ps1  copy-based command registration
```

OS behavior stays behind `MaintenanceRunner`, so orchestration is unit-
testable with mocks — no destructive ops run during `cargo test`.

## Commands

| Command      | Action                                            | Maintenance? |
| ------------ | ------------------------------------------------- | ------------ |
| `akari`      | Interactive TUI home (logo, commands, animation)  | No — idle    |
| `akariclean` | Cleanup phase only ([1/18]–[18/18])               | Cleanup only |
| `akarihealth`| System-image repair only ([Repair 1/5]–[5/5])     | Repair only  |
| `akariall`   | Cleanup, then repair, then combined summary       | Both, in order |
| `akariabout` | About (name, V00.1, purpose, repo/license)        | No           |
| `akarihelp`  | Usage, per-command help, exit instructions        | No           |

`akari <name>` also works (`akari clean`, `akari --help`, …).
In-TUI keys: `1–6` switch command · `c` cleanup · `r` repair · `t` all ·
`?` help · `a` about · `h` home · `↑/↓` scroll · `q`/`Esc` quit ·
`Ctrl+C` deliberate cancel/quit (double-press while running).

## Sprite assets

Directory: `assets/akari/sweep/` (shipped to `<InstallDir>\assets\…`).

```
akari_sweep_001.png … akari_sweep_008.png   (each 256×320, 32bpp ARGB)
```

Validated at startup: all 8 must decode at exactly 256×320, in order,
looped while cleanup runs (~180 ms/frame). Missing/corrupt frames produce
an explicit diagnostic in the animation panel — the TUI stays usable.

## Rendering + fallback

Terminals can't blit 256×320 pixels, so frames render as true-color
half-block cells (`▀` = 2 vertical px/cell), aspect-preserved, centered
(no per-frame drift), transparency blended to background. Caption always
labels it a terminal approximation — never "1:1 pixels". No image-protocol
dependency; works in stock Windows Terminal / conhost. Small terminals
(<60×18) fall back to a compact single-column view instead of overlapping.

## Build

```powershell
cargo fmt --check
cargo check
cargo test
cargo build --release   # -> target\release\akari-sweeps.exe
```

## Install (Windows command registration)

Chosen strategy: **one binary copied under six names** + exe-stem dispatch.
No permanent BAT wrapper; each alias is native (CWD-independent, args
forwarded, exit codes native).

```powershell
.\install.ps1                      # build + install to %LOCALAPPDATA%\Akari-Sweeps
.\install.ps1 -InstallDir "D:\Tools\Akari"   # custom location
```

Adds the install dir to the **user** PATH (machine PATH untouched).
Restart the terminal, then `akarihelp` should open the TUI.

### Uninstall / undo

```powershell
.\uninstall.ps1                    # removes exes, assets, and the PATH entry
```

## Admin requirements

Cleanup + repair need Administrator (BAT's `fltmc` probe is mirrored in
`system::windows::is_elevated`). Without elevation the app warns with
re-launch instructions and continues — privileged steps then fail honestly
with their real exit codes rather than fake success.

## Errors & exit codes

Summary rule mirrors BAT `:ShowSummary`: errors + some success → `1`;
errors + zero success → `2`; clean → `0`. DISM `3010` = success + "restart
required" warning. CHKDSK `0/1/2/3+` mapped per BAT. Locked files are
skipped + counted (`files_left`), unmeasurable folders flagged, Recycle Bin
/ cleanmgr / SilentCleanup reported as "issued, amount not reported" —
never invented.

## Known limitations

- Repair needs Windows 10 servicing stack + elevation; on non-Windows
  hosts the TUI runs but Windows-only subprocesses fail honestly.
- Disk-space deltas include concurrent drive activity (same caveat as BAT).
- Animation is a scaled approximation, not pixel-exact blitting.
- `akariall` runs repair without a second Y/N gate — the explicit alias
  is treated as consent (documented deviation from the BAT's prompt).

## Future animation sets

`assets/akari/{idle,health,about,help}/` are reserved. Add frames +
extend `akari::animation::AnimationSet`; the TUI panel needs no rewrite.

## Tests

```powershell
cargo test   # 43 tests: dispatch, phase order, exit mapping, mocks, sprites
```

Destructive/real servicing ops never run in tests (mock runners + temp
dirs only). TUI resize/exit-code/shim-CWD cases are covered by unit +
layout-guard tests; interactive screenshot runs remain manual.
