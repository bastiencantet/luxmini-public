#!/usr/bin/env python3
"""Turn assets/icon-source.png into assets/Icon.icns.

Steps:
  1. Trim opaque-bleed/white borders so the icon squircle hugs its bounds.
  2. Resize the content to 824x824 and paste onto a 1024x1024 transparent
     canvas (Apple's macOS Big Sur+ template: 10% air on each side).
  3. Emit every .iconset size and call `iconutil` to produce Icon.icns.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets" / "icon-source.png"
ICONSET = ROOT / "assets" / "Icon.iconset"
ICNS = ROOT / "assets" / "Icon.icns"

CONTENT_SIZE = 1024
CANVAS_SIZE = 1024

# iconutil-expected sizes + filenames
SIZES = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]


def trim_to_content(img: Image.Image) -> Image.Image:
    """Trim white/transparent padding around the visible icon."""
    img = img.convert("RGBA")
    pixels = img.load()
    w, h = img.size

    def is_padding(px):
        r, g, b, a = px
        if a < 10:
            return True
        # Near-white
        return r > 245 and g > 245 and b > 245

    # Scan rows/cols
    def first_non_padding_row(start, stop, step):
        for y in range(start, stop, step):
            for x in range(w):
                if not is_padding(pixels[x, y]):
                    return y
        return None

    def first_non_padding_col(start, stop, step):
        for x in range(start, stop, step):
            for y in range(h):
                if not is_padding(pixels[x, y]):
                    return x
        return None

    top = first_non_padding_row(0, h, 1)
    bottom = first_non_padding_row(h - 1, -1, -1)
    left = first_non_padding_col(0, w, 1)
    right = first_non_padding_col(w - 1, -1, -1)
    if None in (top, bottom, left, right):
        return img
    return img.crop((left, top, right + 1, bottom + 1))


def main():
    if not SOURCE.exists():
        print(f"missing source: {SOURCE}", file=sys.stderr)
        sys.exit(1)

    src = Image.open(SOURCE).convert("RGBA")
    trimmed = trim_to_content(src)
    print(f"trimmed: {src.size} -> {trimmed.size}")

    # Fit into CONTENT_SIZE x CONTENT_SIZE while preserving aspect ratio.
    tw, th = trimmed.size
    scale = CONTENT_SIZE / max(tw, th)
    new_size = (round(tw * scale), round(th * scale))
    trimmed = trimmed.resize(new_size, Image.LANCZOS)

    canvas = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    paste_x = (CANVAS_SIZE - new_size[0]) // 2
    paste_y = (CANVAS_SIZE - new_size[1]) // 2
    canvas.paste(trimmed, (paste_x, paste_y), trimmed)
    print(f"content placed at {paste_x},{paste_y}")

    if ICONSET.exists():
        shutil.rmtree(ICONSET)
    ICONSET.mkdir(parents=True)

    for name, size in SIZES:
        out = canvas.resize((size, size), Image.LANCZOS)
        out.save(ICONSET / name, "PNG")

    print(f"wrote {len(SIZES)} PNGs to {ICONSET.name}/")

    # Convert iconset to .icns
    if ICNS.exists():
        ICNS.unlink()
    subprocess.run(
        ["iconutil", "-c", "icns", "-o", str(ICNS), str(ICONSET)],
        check=True,
    )
    size = os.path.getsize(ICNS)
    print(f"wrote {ICNS.name} ({size // 1024} KB)")


if __name__ == "__main__":
    main()
