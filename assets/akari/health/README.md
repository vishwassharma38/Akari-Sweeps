# Akari health frames

Nurse-inspired health-assistant frames shown during health/repair
maintenance (including the health phase of `akari all`). Every numbered
`health_<digits>.png` file in this directory is part of the animation.

Frames are discovered automatically: every `health_<digits>.png` file is
loaded in numeric order, with no hardcoded frame limit. Every file is kept
as its own frame; byte-identical duplicates are not deduplicated. Non-PNG
files are ignored, while numeric gaps, duplicate indices, and PNG files
that do not follow the naming convention are reported as explicit
diagnostics. Playback uses the number of frames that actually loaded, so
additional frames extend the loop without a source-code change.

Replacement PNGs may use any dimensions and aspect ratio; frames within
this set may even differ in size. RGB and RGBA PNGs are accepted and alpha
transparency is preserved. The current art is opaque RGB, so its dark
background is part of the artwork. At startup the loader measures every
frame, builds a shared transparent canvas from the largest width and
height, centers each image on it without stretching or cropping, and
renders the set proportionally in the terminal. Source PNGs are never
modified.

To replace this animation, overwrite these files (or add the next numbered
frames) and keep a contiguous 1-based sequence, then build and run the
application. No manual resizing or dimension configuration is required.
