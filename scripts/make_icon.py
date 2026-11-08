#!/usr/bin/env python3
"""Turn assets/icon-source.png into assets/Icon.icns.

Steps:
  1. Trim transparent padding from the RGBA source, preserving white artwork.
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

CONTENT_SIZE = 824
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
    """Use the source alpha as the silhouette; never remove white LED pixels."""
    img = img.convert("RGBA")
    alpha = img.getchannel("A")
    if alpha.getextrema()[0] == 255:
        raise ValueError("icon source is opaque; remove its background before building")
    # Ignore almost invisible export specks when measuring the silhouette;
    # keep a two-pixel antialias fringe in the crop itself.
    bounds = alpha.point(lambda value: 255 if value >= 32 else 0).getbbox()
    if bounds is None:
        raise ValueError("icon source is fully transparent")
    left, top, right, bottom = bounds
    return img.crop((
        max(0, left - 2),
        max(0, top - 2),
        min(img.width, right + 2),
        min(img.height, bottom + 2),
    ))


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
    # Composite once: passing alpha as a paste mask would apply it twice and
    # darken the antialiased rim.
    canvas.alpha_composite(trimmed, (paste_x, paste_y))
    print(f"content placed at {paste_x},{paste_y}")

    if ICONSET.exists():
        shutil.rmtree(ICONSET)
    ICONSET.mkdir(parents=True)

    for name, size in SIZES:
        out = canvas.resize((size, size), Image.LANCZOS)
        alpha = out.getchannel("A")
        corners = ((0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1))
        if any(alpha.getpixel(point) != 0 for point in corners):
            raise ValueError(f"{name}: icon corners must be transparent")
        if alpha.getpixel((size // 2, size // 2)) < 250:
            raise ValueError(f"{name}: the white LED must not be cut out")
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
