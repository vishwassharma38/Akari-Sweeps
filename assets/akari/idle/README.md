# Akari idle frames

Eight book-reading frames shown while the Home screen waits for input:

- `idle_001.png` … `idle_008.png`

Replacement PNGs may use any dimensions and aspect ratio; frames within
this set may even differ in size. RGB and RGBA PNGs are accepted and alpha
transparency is preserved. The current art is opaque RGB, so its dark
background is part of the artwork. At startup the loader measures every
frame, builds a shared transparent canvas from the largest width and
height, centers each image on it without stretching or cropping, and
renders the set proportionally in the terminal. Source PNGs are never
modified.

To replace this animation, overwrite these eight files, keep their names
and order, then build and run the application. No manual resizing or
dimension configuration is required.
