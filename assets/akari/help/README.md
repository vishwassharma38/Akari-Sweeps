# Akari help assets

All eight files are loaded: `help_001.png`–`help_008.png` play in numeric
order on the Help screen as the ambient help animation. Files
`help_005.png`–`help_008.png` are currently byte-identical duplicates of
frames 001–004, but every file is kept as a separate frame — duplicates
are never detected or skipped, so they intentionally extend the loop to
eight frames.

Frames are discovered automatically: every `help_<digits>.png` file is
loaded with no hardcoded frame limit. Leading zeros are optional, and
additional frames play without a source-code change. Non-PNG files are
ignored, while numeric gaps, duplicate indices, and PNG files that do not
follow the naming convention are reported as explicit diagnostics.

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
