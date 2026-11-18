# Akari help assets

Four unique poses in `help_001.png`–`help_004.png`, played on the Help
screen as the ambient help animation. Files `help_005.png`–`help_008.png`
are byte-identical duplicates and are not loaded.

Replacement PNGs may use any dimensions and aspect ratio; frames within
this set may even differ in size. RGB and RGBA PNGs are accepted and alpha
transparency is preserved. The current art is opaque RGB, so its dark
background is part of the artwork. At startup the loader measures every
frame, builds a shared transparent canvas from the largest width and
height, centers each image on it without stretching or cropping, and
renders the set proportionally in the terminal. Source PNGs are never
modified.

To replace this animation, overwrite these four files, keep their names
and order, then build and run the application. No manual resizing or
dimension configuration is required.
