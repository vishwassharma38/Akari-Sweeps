# Akari-Sweeps_V00.1

Windows maintenance TUI in Rust — a faithful port of `Akari-Sweeps.bat`
(the behavioral spec) with Akari's sweeping animation in a persistent
terminal interface. The Rust app is the implementation of record; the BAT
file remains as the auditable baseline, not the runtime engine.

## Purpose

- Reproduce the BAT's junk-cleanup targets ([1/18]–[18/18] directories,
  npm/pip, DNS/Recycle Bin, Disk Cleanup/SilentCleanup) and Windows
  servicing/repair commands with identical arguments and exit-code
  semantics, under strict cleaning/repair separation (see below).
  Note: `DISM /StartComponentCleanup` historically ran as BAT `[18/18]`
  cleanup; it now runs as the first health step so `akariclean`
  launches zero DISM commands.
- Present live output + per-operation results in a responsive Ratatui TUI
  while Akari sweeps in a dedicated right-side panel.
- Never claim success from "command launched" — only from measured results
  and real exit codes.

## Architecture

```
src/main.rs          entry, exe-name dispatch, elevation startup, event loop
src/app.rs           App state (screen, execution, log, ops, input, animator)
src/commands/mod.rs  command registry, 5-item selection, input mapping,
                     phase identity (Phase) + capability gates (PhaseGate)
src/commands/clean.rs cleanup plan ([1/18]–[18/18] dirs/programs, no DISM) + executor
src/commands/health.rs health plan (DISM StartComponentCleanup + Check/Scan/Restore, SFC, CHKDSK) + exit-code mapping
src/commands/all.rs  akariall sequencing (cleanup, then repair, merged summary)
src/commands/runner.rs RealRunner + RecordingRunner (isolation tests)
src/system/mod.rs    OpStatus/OpResult/ExecEvent/MaintenanceRunner/AppSummary
src/system/elevation.rs UAC decision, runas relaunch, cancel handling
src/system/process.rs run_program_capture(), program_exists()
src/system/windows.rs is_elevated() (fltmc, like BAT), free_space_kb()
src/akari/sprites.rs sweep discovery/validation/loading (256×320 RGBA)
src/akari/animation.rs Animator (Ambient/Sweeping/Settled), 6 FPS deadline
src/ui/*             Ratatui layout: header, main/output, animation, entry
install.ps1 / uninstall.ps1  copy-based command registration
```

OS behavior stays behind `MaintenanceRunner`, so orchestration is unit-
testable with mocks — no destructive ops run during `cargo test`.

## Commands

`akari` launches the interface. Inside it, exactly five commands are
selectable (keys `1`–`5`):

| Key | Command      | Action                                        | Maintenance? |
| --- | ------------ | --------------------------------------------- | ------------ |
| `1` | `akariclean` | Cleanup only, zero DISM/SFC/CHKDSK            | Cleanup only |
| `2` | `akarihealth`| Health/servicing only ([Repair 1/6]–[6/6])   | Repair only  |
| `3` | `akariall`   | Cleanup, then health, then combined summary   | Both, in order |
| `4` | `akariabout` | About (name, V00.1, purpose, repo/license)    | No           |
| `5` | `akarihelp`  | Usage, per-command help, exit instructions    | No           |

`akari <name>` also works (`akari clean`, `akari --help`, …).

### Command-entry field

Every screen shows a `Command` box. Press `/` (or Enter) to type, then
Enter to run. Input is trimmed and case-insensitive; number selection
and text entry share one dispatch path. Empty input does nothing;
unknown names get a hint listing the valid commands. While typing,
shortcuts suspend except Enter (run), Esc (cancel), Backspace (edit),
and Ctrl+U (clear).

Other keys: `c` cleanup · `r` repair · `t` all · `?` help · `a` about ·
`h` home · `↑/↓` scroll · `q`/`Esc` quit · `Ctrl+C` deliberate cancel/quit
(double-press while running).

### Cleanup/repair isolation (guaranteed, strict)

- `akariclean` runs only cleanup. It invokes zero DISM commands of any
  kind (including `/StartComponentCleanup`), zero SFC, zero CHKDSK.
  `[18/18]` is Disk Cleanup + SilentCleanup only.
- `akarihealth` runs only the six health/servicing commands (DISM
  StartComponentCleanup, Check/Scan/RestoreHealth, SFC, CHKDSK). It
  deletes no files and invokes no cleanup.
- `akariall` runs cleanup to completion, then repair — never concurrent,
  never a second cleanup. **Failure policy:** cleanup errors never block
  repair, but they poison the combined summary, so overall status/exit
  code can never report success when a required phase failed.
- Enforcement is structural: phase runners require capability tokens
  (`CleanupGate`/`RepairGate`) minted only for permitted commands, plus
  invocation-log regression tests asserting the actual command order.

## Administrator elevation (UAC)

Detection uses the BAT's `fltmc` probe — a real privilege check.

- Maintenance aliases (`akariclean`, `akarihealth`, `akariall`) started
  without elevation relaunch the **same executable** with the **same
  arguments** through the Shell `runas` verb
  (`Start-Process -Verb RunAs -Wait`), so aliases keep working after
  elevation. Paths with spaces are quoted; assets resolve from the exe
  directory; PATH is untouched.
- The relaunch is synchronous: the parent propagates the elevated
  child's exit code. Declining UAC yields exit `1223` with an actionable
  message — maintenance is never started, never reported as started.
- An `ELEVATED` marker prevents loops: a relaunched copy that is still
  not elevated aborts with instructions instead of relaunching again.
- Informational screens (home/about/help) need no elevation. From inside
  a non-elevated TUI, maintenance requests elevation in a background
  thread (UI stays responsive); declining logs a message and runs nothing.

### Manual Windows verification (UAC can't be automated in CI)

1. Non-admin terminal: `akariclean` → approve UAC → cleanup runs elevated.
2. Same, but decline UAC → "Elevation declined … was not started.",
   exit `1223`, no maintenance output.
3. `akari` → home works unelevated; press `c` → elevation requested,
   declining keeps the TUI usable with an explanatory log line.
4. `akarihelp` works fully unelevated.

## Sprite assets

Directory: `assets/akari/sweep/` (shipped to `<InstallDir>\assets\…`).

```
akari_sweep_001.png … akari_sweep_008.png   (each 256×320, 32bpp ARGB)
```

Validated at startup: all 8 must decode at exactly 256×320, in order.
Missing/corrupt frames produce an explicit diagnostic in the animation
panel — the TUI stays usable. Source PNGs are never modified.

## Rendering + animation

- Each half-block cell half is an **area average** over its source pixels
  (box filter): gradients survive downscaling instead of aliasing, and
  alpha composites over the terminal background during averaging.
- Aspect preserved, centered on both axes (no per-frame drift).
- True-color RGB output; terminals without true color degrade gracefully
  to shape-only rendering (no crash, no claim of pixel-exact output).
- **6 FPS** (≈167 ms/frame) for all animations via deadline scheduling on
  a monotonic clock: no cumulative drift, stalls resync instead of
  queuing backlog, redraws only on change, input polled within ≤50 ms —
  no busy loop, keyboard never blocked.

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

## Errors & exit codes

Summary rule mirrors BAT `:ShowSummary`: errors + some success → `1`;
errors + zero success → `2`; clean → `0`. Warnings (e.g. undocumented
`cleanmgr` codes, unverifiable Recycle Bin result) show as `!` and never
affect the exit code. DISM `3010` = success + "restart required" warning.
CHKDSK `0/1/2/3+` mapped per BAT. Locked files are skipped + counted
(`files_left`), unmeasurable folders flagged.

## Known limitations

- Repair needs Windows 10 servicing stack + elevation; on non-Windows
  hosts the TUI runs but Windows-only subprocesses fail honestly, and
  elevation is skipped (dev/test path) with a notice.
- UAC interaction itself can't be covered by automated tests (manual
  procedure above); the decision/relaunch/quote logic is unit-tested.
- Disk-space deltas include concurrent drive activity (same caveat as BAT).
- Animation is a scaled approximation, not pixel-exact blitting.
- `akariall` runs repair without a second Y/N gate — the explicit alias
  is treated as consent (documented deviation from the BAT's prompt).

## Future animation sets

`assets/akari/{idle,health,about,help}/` are reserved. Add frames +
extend `akari::animation::AnimationSet`; the TUI panel needs no rewrite.

## Tests

```powershell
cargo test   # 110 tests: dispatch, input/back-nav, placeholder, elevation,
             # strict isolation (zero-DISM clean, 6-step health, ordering),
             # npm/Recycle/DISM-740/filesystem, sprites, rendering, exit codes
             # (mock runners + temp dirs only)
```

Destructive/real servicing ops never run in tests. TUI resize/exit-code
cases are covered by TestBackend layout tests; UAC prompts and live
maintenance runs remain manual (see verification procedure).
