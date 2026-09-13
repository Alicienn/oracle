"""Generates the installer's header and sidebar artwork.

NSIS accepts only 24-bit BMP for these, at exact sizes, with no alpha channel — so they
cannot be produced by the same sharp pipeline as the application icons, and they cannot be
the PNG mark dropped in as-is. This composes both from `assets/mark.png` over the
application's own paper colour, flattening transparency rather than letting NSIS render it
as black.

Run after changing the mark:

    python scripts/make-installer-art.py
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MARK = ROOT / "assets" / "mark.png"
OUT = ROOT / "src-tauri" / "installer"

# From tokens.css: the light theme's paper, and the accent the mark is drawn in.
PAPER = (244, 243, 238)
PAPER_DEEP = (234, 232, 224)

# Sizes MUI2 expects. Anything else is letterboxed or rejected outright.
HEADER = (150, 57)
SIDEBAR = (164, 314)


def mark(size: int) -> Image.Image:
    """The mark, resized, still carrying its alpha."""
    return Image.open(MARK).convert("RGBA").resize((size, size), Image.LANCZOS)


def header() -> Image.Image:
    """Small strip in the corner of every page: the mark, and room to breathe."""
    canvas = Image.new("RGB", HEADER, PAPER)
    glyph = mark(40)

    # Right-aligned, which is where MUI2's header text is not.
    canvas.paste(glyph, (HEADER[0] - glyph.width - 10, (HEADER[1] - glyph.height) // 2), glyph)
    return canvas


def sidebar() -> Image.Image:
    """The welcome and finish page panel.

    A vertical wash from paper to its deeper tone, so the panel reads as part of the
    application rather than a white rectangle with a logo in it.
    """
    canvas = Image.new("RGB", SIDEBAR, PAPER)
    pixels = canvas.load()

    for y in range(SIDEBAR[1]):
        ratio = y / (SIDEBAR[1] - 1)
        row = tuple(
            round(PAPER[channel] + (PAPER_DEEP[channel] - PAPER[channel]) * ratio)
            for channel in range(3)
        )
        for x in range(SIDEBAR[0]):
            pixels[x, y] = row

    glyph = mark(96)
    # Above centre: the finish page puts its text in the lower half.
    canvas.paste(glyph, ((SIDEBAR[0] - glyph.width) // 2, 58), glyph)

    return canvas


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)

    for name, image in (("header.bmp", header()), ("sidebar.bmp", sidebar())):
        path = OUT / name
        # 24-bit, no colour table: what NSIS reads and nothing more.
        image.convert("RGB").save(path, format="BMP")
        print(f"{path.relative_to(ROOT)}  {image.size[0]}x{image.size[1]}")


if __name__ == "__main__":
    main()
