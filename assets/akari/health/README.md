# Akari health frames

Eight nurse-inspired health-assistant frames shown during health/repair
maintenance (including the health phase of `akariall`):

- `health_001.png` — `health_008.png` (each 358×575, RGB, dark background)

The Rust app validates all eight in order and loads them through the
shared sprite pipeline without modifying the source PNGs. The frames are
opaque (no alpha); the loader normalizes them to RGBA for the shared
renderer.
