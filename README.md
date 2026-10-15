# AkariSweeps

Minimal Rust project scaffold for AkariSweeps.

## Current scope

This is only the initial project structure. The maintenance workflows and TUI behavior have not been implemented yet.

## Development

Install the Rust toolchain, then run:

```powershell
cargo check
cargo run
```

Keep your existing `Akari-Sweeps.bat` in the project root. This scaffold does not replace or modify it; we can adapt the launcher after you share the baseline BAT script.

## Akari sweep sprite assets

Place the eight processed sweep frames in `assets/akari/sweep/`.

Expected names, with your actual image extension (for example `.png`):

- `akari_sweep_001.png`
- `akari_sweep_002.png`
- `akari_sweep_003.png`
- `akari_sweep_004.png`
- `akari_sweep_005.png`
- `akari_sweep_006.png`
- `akari_sweep_007.png`
- `akari_sweep_008.png`

If your exported files use a different extension, keep that extension consistently. Image loading and terminal rendering are not wired up yet.
