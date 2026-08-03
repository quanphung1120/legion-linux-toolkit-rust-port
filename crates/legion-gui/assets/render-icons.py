#!/usr/bin/env python3
"""Pre-render the tray icons.

One icon per power profile: the app logo with a status dot tinted the colour
Lenovo's power-button LED shows in that mode. The outputs are committed, so
this only needs re-running when logo.png or the colour mapping changes.

The colours must stay in sync with `LedColor::hex()` in
crates/legion-hw/src/profile.rs, and the file names with
`state::tray_icon_name()`.
"""

import os

from PIL import Image, ImageDraw

COLORS = {
    "quiet": (0x3B, 0x82, 0xF6),        # low-power  — blue
    "balanced": (0xE5, 0xE7, 0xEB),     # balanced   — white
    "performance": (0xEF, 0x44, 0x44),  # performance — red
    "extreme": (0xA8, 0x55, 0xF7),      # max-power  — purple
    "custom": (0xA8, 0x55, 0xF7),       # custom     — purple
}
SIZE = 64

here = os.path.dirname(os.path.abspath(__file__))
base = Image.open(os.path.join(here, "logo.png")).convert("RGBA")
base = base.resize((SIZE, SIZE), Image.LANCZOS)
base.save(os.path.join(here, "legion-toolkit.png"))
print("wrote legion-toolkit.png")

for name, rgb in COLORS.items():
    img = base.copy()
    draw = ImageDraw.Draw(img)
    radius = 11
    cx = cy = SIZE - radius - 3
    # A dark ring keeps the dot legible on both light and dark panels.
    draw.ellipse(
        [cx - radius - 2, cy - radius - 2, cx + radius + 2, cy + radius + 2],
        fill=(18, 20, 26, 255),
    )
    draw.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=rgb + (255,))
    out = f"legion-toolkit-{name}.png"
    img.save(os.path.join(here, out))
    print("wrote", out)
