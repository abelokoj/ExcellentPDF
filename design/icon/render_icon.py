"""Draws the ExcellentPDF mark ("Marginalia", see philosophy.md) at every icon size, each size drawn for its own
pixel grid rather than shrunk, and writes the PNGs, the Windows .ico and the specimen sheet.

    python design/icon/render_icon.py

Needs Pillow. Geometry is in units of a 256 square; each size is drawn supersampled and box-reduced.
"""

import math
import os
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

PAPER = (251, 250, 246, 255)
FLAP = (233, 229, 219, 255)
INK = (26, 61, 143, 255)
NIB_ANGLE = math.radians(35)

# Sizes the .ico carries (Windows picks per DPI), and the PNGs kept for the other platforms.
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
PNG_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024]

# The cursive "e" that runs out into an underline: cubic segments, units.
E_PATH = [
    ((58, 186), (82, 178), (120, 162), (117, 146)),
    ((117, 146), (115, 124), (80, 124), (77, 152)),
    ((77, 152), (74, 177), (97, 199), (131, 198)),
    ((131, 198), (166, 197), (198, 187), (232, 168)),
]
# At sixteen pixels the loop would close up: only the swash is drawn.
SWASH_PATH = [((54, 200), (110, 204), (170, 196), (236, 166))]


def tier(size):
    """What changes with the pixel grid: the page box, the fold, the text lines and the pen."""
    if size <= 20:
        return dict(box=(36, 12, 220, 244), fold=64, lines=[(0.36, 0.86), (0.52, 0.70)], line_px=1, path=SWASH_PATH,
                    nib=(46, 26), edge=(140, 147, 158, 255), text=(168, 174, 184, 255), shadow=False)
    if size <= 32:
        return dict(box=(36, 12, 220, 244), fold=60, lines=[(0.32, 0.86), (0.44, 0.74), (0.56, 0.82)], line_px=1,
                    path=E_PATH, nib=(32, 13), edge=(150, 157, 167, 255), text=(178, 184, 193, 255), shadow=False)
    return dict(box=(44, 18, 212, 238), fold=46, lines=[(0.24, 0.70), (0.32, 0.86), (0.40, 0.60)],
                line_px=None, path=E_PATH, nib=(20, 3.6), edge=(185, 190, 198, 255), text=(196, 201, 208, 255),
                shadow=True)


def snap(u, size):
    """A unit coordinate moved onto the nearest pixel edge of a `size` grid."""
    return math.floor(u * size / 256 + 0.5) * 256 / size


def bezier(p0, p1, p2, p3, steps):
    for i in range(steps + 1):
        t = i / steps
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t, t ** 3
        yield (a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1])


def ink_points(path):
    pts = []
    for seg in path:
        part = list(bezier(*seg, 200))
        pts.extend(part if not pts else part[1:])
    return pts


def draw(size):
    t = tier(size)
    ss = max(2, min(64, 2048 // size))
    k = size * ss / 256  # canvas pixels per unit
    px_u = 256 / size  # one output pixel, in units
    canvas = Image.new("RGBA", (size * ss, size * ss), (0, 0, 0, 0))

    x0, y0, x1, y1 = (snap(v, size) for v in t["box"])
    f = snap(t["fold"], size)
    radius = max(px_u * 0.75, 9) * k
    s = lambda pts: [(x * k, y * k) for x, y in pts]

    edge_u = px_u if size <= 64 else 1.6

    def page_mask(e):
        """The page inset by `e` units: a rounded sheet with its corner cut on the fold's diagonal."""
        m = Image.new("L", canvas.size, 0)
        d = ImageDraw.Draw(m)
        d.rounded_rectangle([(x0 + e) * k, (y0 + e) * k, (x1 - e) * k - 1, (y1 - e) * k - 1],
                            radius=max(radius - e * k, 1), fill=255)
        r2 = e * (math.sqrt(2) - 1)
        d.polygon(s([(x1 - f + r2 - 8, y0 - 8), (x1 + 8, y0 - 8), (x1 + 8, y0 + f - r2 + 8)]), fill=0)
        return m

    def flap_mask(e):
        """The turned-down corner, inset by `e` units."""
        m = Image.new("L", canvas.size, 0)
        r2 = e * math.sqrt(2)
        ImageDraw.Draw(m).polygon(s([(x1 - f + e, y0 + e + r2), (x1 - e - r2, y0 + f - e), (x1 - f + e, y0 + f - e)]),
                                  fill=255)
        return m

    page, inner = page_mask(0), page_mask(edge_u)

    if t["shadow"]:
        shade = Image.new("L", canvas.size, 0)
        shade.paste(page, (0, round(3.5 * k)))
        shade = shade.filter(ImageFilter.GaussianBlur(6 * k)).point(lambda v: v * 0.30)
        canvas.paste(Image.new("RGBA", canvas.size, (24, 32, 46, 255)), (0, 0), shade)

    canvas.paste(Image.new("RGBA", canvas.size, t["edge"]), (0, 0), page)
    canvas.paste(Image.new("RGBA", canvas.size, PAPER), (0, 0), inner)

    # The text: bars on a measure with a ragged right edge, kept clear of the fold.
    w, h = x1 - x0, y1 - y0
    left = x0 + w * 0.16
    for top, length in t["lines"]:
        if t["line_px"]:
            cy = (math.floor((y0 + h * top) / px_u) + 0.5) * px_u
            half = t["line_px"] * px_u / 2
            bar = [left, cy - half, snap(x0 + w * length, size), cy + half]
            ImageDraw.Draw(canvas).rectangle([v * k for v in bar], fill=t["text"])
        else:
            cy, half = y0 + h * top, 3.6
            right = min(x0 + w * length, x1 - f - 10) if top < (f + 10) / h else x0 + w * length
            ImageDraw.Draw(canvas).rounded_rectangle(
                [left * k, (cy - half) * k, right * k, (cy + half) * k], radius=half * k, fill=t["text"])

    # The fold: the corner turned down over the page, its shadow in the crease.
    flap = flap_mask(0)
    if t["shadow"]:
        crease = Image.new("L", canvas.size, 0)
        crease.paste(flap, (round(-1.5 * k), round(2.5 * k)))
        crease = crease.filter(ImageFilter.GaussianBlur(3 * k)).point(lambda v: v * 0.22)
        crease = ImageChops.multiply(crease, inner)
        canvas.paste(Image.new("RGBA", canvas.size, (24, 32, 46, 255)), (0, 0), crease)
    canvas.paste(Image.new("RGBA", canvas.size, t["edge"]), (0, 0), flap)
    canvas.paste(Image.new("RGBA", canvas.size, FLAP), (0, 0), flap_mask(edge_u))

    # The ink: a broad nib held at 35 degrees, its width along the nib, tapered in and out by the hand's pressure.
    ink = Image.new("L", canvas.size, 0)
    di = ImageDraw.Draw(ink)
    wide, thin = t["nib"]
    nib = (math.cos(NIB_ANGLE), -math.sin(NIB_ANGLE))
    pts = ink_points(t["path"])
    n = len(pts) - 1
    prev = None
    for i, (x, y) in enumerate(pts):
        u = i / n
        press = min(1.0, 0.35 + u / 0.06 * 0.65, 0.18 + (1 - u) / 0.28 * 0.82)
        half = wide * press / 2
        a, b = (x + nib[0] * half, y + nib[1] * half), (x - nib[0] * half, y - nib[1] * half)
        r = thin * max(press, 0.5) / 2
        di.ellipse([(x - r) * k, (y - r) * k, (x + r) * k, (y + r) * k], fill=255)
        if prev:
            di.polygon(s([prev[0], a, b, prev[1]]), fill=255)
        prev = (a, b)
    canvas.paste(Image.new("RGBA", canvas.size, INK), (0, 0), ink)

    return canvas.reduce(ss)


def font(name, px):
    fonts = Path(os.environ.get("CANVAS_FONTS", ""))
    try:
        return ImageFont.truetype(str(fonts / name), px)
    except OSError:
        return ImageFont.load_default(px)


def sheet(icons, out):
    """Plate I: the mark large, at every size it ships, on light and dark, and its smallest grids magnified."""
    W, H = 2400, 1500
    bg, ink_grey, rule = (244, 242, 236, 255), (92, 98, 108, 255), (206, 203, 194, 255)
    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)
    label = font("DMMono-Regular.ttf", 22)
    title = font("Jura-Light.ttf", 34)
    small = font("DMMono-Regular.ttf", 18)

    big = draw(1024).resize((880, 880), Image.LANCZOS)
    img.alpha_composite(big, (120, 230))
    d.text((140, 120), "M A R G I N A L I A", font=title, fill=(40, 46, 56, 255))
    d.text((140, 172), "PLATE I: THE EXCELLENTPDF MARK", font=small, fill=ink_grey)
    d.line([(1080, 150), (1080, H - 150)], fill=rule, width=1)

    # Every shipped size at its true pixels, on paper and on a dark taskbar.
    sizes = [256, 128, 64, 48, 32, 24, 16]
    x, base = 1160, 560
    for sz in sizes:
        img.alpha_composite(icons[sz], (x, base - sz))
        d.text((x, base + 22), f"{sz}", font=label, fill=ink_grey)
        x += sz + 46
    dark = (1160, 650, 1160 + 40 + sum(s + 64 for s in [64, 48, 40, 32, 24, 20, 16]) - 24, 790)
    d.rectangle(dark, fill=(32, 32, 34, 255))
    x = 1200
    for sz in [64, 48, 40, 32, 24, 20, 16]:
        img.alpha_composite(icons[sz], (x, dark[3] - 30 - sz))
        x += sz + 64
    d.text((1160, 806), "ON A DARK TASKBAR", font=small, fill=ink_grey)

    # The smallest grids, magnified, with their pixel lattice.
    x = 1160
    for sz, mag in [(16, 20), (24, 14), (32, 10)]:
        plate = icons[sz].resize((sz * mag, sz * mag), Image.NEAREST)
        y = 900
        d.rectangle([x, y, x + sz * mag, y + sz * mag], fill=(255, 255, 255, 255))
        img.alpha_composite(plate, (x, y))
        for i in range(sz + 1):
            d.line([(x + i * mag, y), (x + i * mag, y + sz * mag)], fill=(225, 222, 214, 255), width=1)
            d.line([(x, y + i * mag), (x + sz * mag, y + i * mag)], fill=(225, 222, 214, 255), width=1)
        d.text((x, y + 336 + 18), f"{sz} PX  x{mag}", font=small, fill=ink_grey)
        x += sz * mag + 60

    d.text((140, H - 110), "page: set  /  ink: written", font=small, fill=ink_grey)
    img.convert("RGB").save(out, optimize=True)


def main():
    icons = {sz: draw(sz) for sz in sorted(set(PNG_SIZES) | set(ICO_SIZES))}
    png_dir = HERE / "png"
    png_dir.mkdir(exist_ok=True)
    for sz in PNG_SIZES:
        icons[sz].save(png_dir / f"excellentpdf-{sz}.png", optimize=True)
    ico = ROOT / "packaging" / "windows" / "excellentpdf.ico"
    first = icons[ICO_SIZES[-1]]
    first.save(ico, format="ICO", sizes=[(s, s) for s in ICO_SIZES],
               append_images=[icons[s] for s in ICO_SIZES[:-1]])
    sheet(icons, HERE / "excellentpdf-icon-sheet.png")
    print("wrote", ico)


if __name__ == "__main__":
    main()
