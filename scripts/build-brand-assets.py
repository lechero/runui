#!/usr/bin/env python3
"""Export runui's SVG mark as browser icons and a social preview card.

Requires Pillow and rsvg-convert (librsvg). The SVGs in docs/assets are the
source artwork; this script only writes raster exports next to them.
"""

from __future__ import annotations

import io
import shutil
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "docs/assets"
MARK = ASSETS / "runui-mark.svg"
CYAN = "#7DD3FC"
GREEN = "#4ADE80"
BG = "#0B1220"
TEXT = "#E5E7EB"
MUTED = "#A2AEC1"


def font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont:
    paths = [
        ("/System/Library/Fonts/Menlo.ttc", 1 if bold else 0),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf" if bold else "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0),
    ]
    for path, index in paths:
        if Path(path).exists():
            return ImageFont.truetype(path, size, index=index)
    raise SystemExit("A monospace font is needed to export the social card.")


def raster_mark(size: int) -> Image.Image:
    result = subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), str(MARK)],
        check=True, capture_output=True,
    )
    return Image.open(io.BytesIO(result.stdout)).convert("RGBA")


def browser_icons() -> Image.Image:
    large = raster_mark(1024)
    for size, name in [
        (16, "favicon-16.png"),
        (32, "favicon-32.png"),
        (180, "apple-touch-icon.png"),
        (512, "runui-icon-512.png"),
    ]:
        large.resize((size, size), Image.Resampling.LANCZOS).save(ASSETS / name, optimize=True)
    large.save(ASSETS / "favicon.ico", format="ICO", sizes=[(16, 16), (32, 32), (48, 48)])
    return large


def social_card(mark: Image.Image) -> None:
    width, height = 1200, 630
    card = Image.new("RGB", (width, height), BG)
    glow = Image.new("RGBA", card.size)
    g = ImageDraw.Draw(glow)
    g.ellipse((650, -240, 1430, 550), fill=(54, 133, 171, 64))
    g.ellipse((870, 290, 1300, 820), fill=(74, 222, 128, 30))
    glow = glow.filter(ImageFilter.GaussianBlur(105))
    card = Image.alpha_composite(card.convert("RGBA"), glow)
    draw = ImageDraw.Draw(card)

    draw.rounded_rectangle((24, 24, 1176, 606), radius=25, outline="#294056", width=2)
    draw.line((78, 515, 1122, 515), fill="#2E435B", width=2)
    draw.text((86, 554), "TASKFILE   PACKAGE.JSON   MAKEFILE   COMPOSE   MAVEN", fill="#8395AB", font=font(18, True))

    icon = mark.resize((70, 70), Image.Resampling.LANCZOS)
    card.alpha_composite(icon, (79, 72))
    draw.text((166, 84), "runui", fill=TEXT, font=font(43, True))
    draw.text((86, 214), "Project commands,", fill=TEXT, font=font(53, True))
    draw.text((86, 290), "ready to run.", fill=CYAN, font=font(53, True))
    draw.text((88, 399), "Humans + agents. Shared runs.", fill=MUTED, font=font(22))

    panel = (748, 174, 1110, 449)
    draw.rounded_rectangle(panel, radius=17, fill="#111C2D", outline="#4B6178", width=2)
    draw.rounded_rectangle((760, 187, 1098, 220), radius=7, fill="#192A3D")
    for n, color in enumerate(("#FB7185", "#FBBF24", GREEN)):
        x = 777 + n * 18
        draw.ellipse((x, 199, x + 8, 207), fill=color)
    draw.text((851, 193), "runui  ·  atlas", fill="#B6C7D8", font=font(15))
    draw.rounded_rectangle((765, 240, 913, 277), radius=6, fill=CYAN)
    draw.text((779, 247), "★ preview", fill=BG, font=font(17, True))
    draw.text((940, 249), "● running", fill=GREEN, font=font(16))
    draw.text((773, 305), "$ task preview", fill=CYAN, font=font(17))
    draw.text((773, 345), "cycle 01  checked 4 packages", fill=TEXT, font=font(14))
    draw.text((773, 375), "cycle 02  checked 4 packages", fill=TEXT, font=font(14))
    draw.text((773, 405), "▏", fill=GREEN, font=font(20, True))

    card.convert("RGB").save(ASSETS / "social-card.png", optimize=True)


def main() -> None:
    if shutil.which("rsvg-convert") is None:
        raise SystemExit("Install librsvg (rsvg-convert) to export the brand assets.")
    large = browser_icons()
    social_card(large)
    print("Exported favicons, app icon, and social card to docs/assets/.")


if __name__ == "__main__":
    main()
