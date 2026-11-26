"""Draws the three proposed ExcellentPDF marks (Spread, Dog-ear, Spectrum; see philosophy.md) at every icon size, each
size drawn for its own pixel grid, and writes their PNGs, a Windows .ico each, and the comparison sheet.

    python design/icon/proposals/render_proposals.py

Needs Pillow. Geometry is in units of a 256 square; each size is drawn supersampled and box-reduced.
"""

import math
import os
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
PNG_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024]


def hexrgb(h, a=255):
    return (int(h[1:3], 16), int(h[3:5], 16), int(h[5:7], 16), a)


class Canvas:
    """A supersampled RGBA canvas addressed in 256-unit coordinates."""

    def __init__(self, size):
        self.size = size
        self.ss = max(2, min(64, 2048 // size))
        self.k = size * self.ss / 256
        self.px = 256 / size  # one output pixel, in units
        self.img = Image.new("RGBA", (size * self.ss,) * 2, (0, 0, 0, 0))

    def pts(self, points):
        return [(x * self.k, y * self.k) for x, y in points]

    def mask(self, points):
        m = Image.new("L", self.img.size, 0)
        ImageDraw.Draw(m).polygon(self.pts(points), fill=255)
        return m

    def fill(self, mask, color, alpha=1.0):
        if alpha < 1:
            mask = mask.point(lambda v: v * alpha)
        self.img.paste(Image.new("RGBA", self.img.size, color), (0, 0), mask)

    def gradient(self, mask, top, bottom, angle=0):
        """Fills `mask` with a linear gradient from `top` to `bottom`, turned `angle` degrees."""
        ramp = Image.linear_gradient("L").rotate(angle, resample=Image.BICUBIC, expand=False)
        ramp = ramp.resize(self.img.size, Image.BICUBIC)
        layer = Image.composite(Image.new("RGBA", self.img.size, bottom), Image.new("RGBA", self.img.size, top), ramp)
        self.img.paste(layer, (0, 0), mask)

    def shadow(self, mask, dx, dy, blur, alpha, color=(18, 24, 38, 255), clip=None):
        s = Image.new("L", self.img.size, 0)
        s.paste(mask, (round(dx * self.k), round(dy * self.k)))
        s = s.filter(ImageFilter.GaussianBlur(blur * self.k)).point(lambda v: v * alpha)
        if clip is not None:
            s = ImageChops.multiply(s, clip)
        self.img.paste(Image.new("RGBA", self.img.size, color), (0, 0), s)

    def line(self, points, width, color):
        d = ImageDraw.Draw(self.img)
        p = self.pts(points)
        w = width * self.k
        d.line(p, fill=color, width=max(1, round(w)), joint="curve")
        for x, y in (p[0], p[-1]):
            d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=color)

    def soft_line(self, points, width, color, alpha):
        """A line blended over what is there (ImageDraw would replace the pixels' alpha)."""
        m = Image.new("L", self.img.size, 0)
        d = ImageDraw.Draw(m)
        p = self.pts(points)
        w = width * self.k
        d.line(p, fill=255, width=max(1, round(w)), joint="curve")
        for x, y in (p[0], p[-1]):
            d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=255)
        self.fill(m, color, alpha)

    def snap(self, u):
        return math.floor(u / self.px + 0.5) * self.px

    def done(self):
        return self.img.reduce(self.ss)


def rrect(x0, y0, x1, y1, r, n=12):
    pts = []
    for cx, cy, a0 in ((x1 - r, y0 + r, -90), (x1 - r, y1 - r, 0), (x0 + r, y1 - r, 90), (x0 + r, y0 + r, 180)):
        for i in range(n + 1):
            a = math.radians(a0 + 90 * i / n)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def rotate(points, deg, pivot):
    a = math.radians(deg)
    c, s = math.cos(a), math.sin(a)
    px, py = pivot
    return [(px + (x - px) * c - (y - py) * s, py + (x - px) * s + (y - py) * c) for x, y in points]


def quad(p0, p1, p2, n=48):
    return [((1 - t) ** 2 * p0[0] + 2 * (1 - t) * t * p1[0] + t * t * p2[0],
             (1 - t) ** 2 * p0[1] + 2 * (1 - t) * t * p1[1] + t * t * p2[1]) for t in (i / n for i in range(n + 1))]


# I. Spread: an open notebook, type on the left leaf, a handwritten line on the right, a vermilion ribbon.
def spread(size):
    c = Canvas(size)
    big, small = size >= 48, size <= 24
    ivory, edge, board, ribbon = hexrgb("#fbf8f1"), hexrgb("#c9c0ae"), hexrgb("#14505c"), hexrgb("#d8432a")

    def leaf(side, dy=0.0, out=0.0):
        """One leaf: its head and foot arc from the outer edge to the spine; `side` -1 left, 1 right."""
        ox = 128 + side * (100 + out)
        head = quad((ox, 60 + dy), (128 + side * 14, 52 + dy), (128, 78 + dy))
        foot = quad((128, 206 + dy), (128 + side * 14, 184 + dy), (ox, 192 + dy))
        return head + foot

    both = lambda dy=0.0, out=0.0: (c.mask(leaf(-1, dy, out)), c.mask(leaf(1, dy, out)))
    cover = ImageChops.lighter(*both(10 if not small else 18, 6 if not small else 9))
    if big:
        c.shadow(cover, 0, 5, 7, 0.30)
    c.fill(cover, board)
    for step in ((6, 3), (3, 1.5)) if not small else ():
        c.fill(ImageChops.lighter(*both(*step)), hexrgb("#e4ddcc"))
        c.fill(ImageChops.lighter(*both(step[0] - 1.2, step[1])), hexrgb("#f4efe3"))
    left, right = both()
    pages = ImageChops.lighter(left, right)
    c.fill(pages, ivory)
    # The gutter: paper darkens as it turns into the spine.
    gutter = c.mask([(116, 30), (140, 30), (140, 230), (116, 230)]).filter(ImageFilter.GaussianBlur(9 * c.k))
    c.fill(ImageChops.multiply(gutter, pages), hexrgb("#d9cfba"), 0.55)
    if not small:
        for side in (-1, 1):
            c.line(leaf(side), max(1.4, c.px * 0.9), edge)
        c.line([(128, 80), (128, 204)], max(1.2, c.px * 0.8), hexrgb("#b9ae98"))

    if big:
        # Type on the left leaf, following the head's arc.
        for i, (start, end) in enumerate([(42, 110), (42, 112), (42, 100), (42, 108), (42, 84)]):
            d = 30 + i * 19
            pts = [(x, y + d) for x, y in quad((28, 60), (114, 52), (128, 78)) if start <= x <= end]
            c.line(pts, 6, hexrgb("#bdb6a8"))
        # One handwritten line on the right leaf.
        # Cursive: a prolate cycloid gives the loops, a slow swell varies their height as a hand does.
        for y, turns in ((100, 5.2), (124, 4.1)):
            u_max = turns * 2 * math.pi
            hand = []
            for i in range(240):
                u = u_max * i / 239
                tall = 1 + 0.35 * math.sin(u * 0.37 + y)
                hand.append((168 + 1.55 * u + 2.6 * math.sin(u), y - 2.2 + 3.4 * tall * math.cos(u) - 0.25 * u))
            c.line(hand, 2.6, hexrgb("#22306a"))
    elif not small:
        for d, end in ((30, 108), (50, 100)):
            y = c.snap(62 + d) + c.px / 2
            c.line([(46, y), (end, y)], c.px, hexrgb("#b0a996"))

    # The ribbon: from the head, across the right leaf, out past the foot, swallow-tailed.
    w = max(14, c.px * 2.4)
    x0 = c.snap(140) if small else 140
    notch = max(8, c.px * 1.6)
    rib = [(x0, 70), (x0 + w, 72), (x0 + w, 242), (x0 + w / 2, 242 - notch), (x0, 242)]
    rm = c.mask(rib)
    if big:
        c.shadow(rm, 2, 2, 2.5, 0.25)
    c.fill(rm, ribbon)
    if big:
        c.fill(c.mask([(x0 + w * 0.62, 72), (x0 + w, 72), (x0 + w, 242), (x0 + w * 0.62, 236)]), hexrgb("#b8331f"), 0.5)
    return c.done()


# II. Dog-ear: one ultramarine page, its corner turned down.
def dogear(size):
    c = Canvas(size)
    big = size >= 48
    x0, y0, x1, y1 = (c.snap(v) for v in (24, 24, 232, 232))
    f = c.snap({16: 96, 20: 90, 24: 86, 32: 80}.get(size, 78))
    r = max(c.px * 2.2, 46)
    tile = c.mask(rrect(x0, y0, x1, y1, r))
    ImageDraw.Draw(tile).polygon(c.pts([(x1 - f - 6, y0 - 6), (x1 + 6, y0 - 6), (x1 + 6, y0 + f + 6)]), fill=0)
    if big:
        c.shadow(tile, 0, 5, 8, 0.32)
    c.gradient(tile, hexrgb("#4a6cff"), hexrgb("#24168a"), angle=-18)
    if big:
        # Type pressed into the page, a tone only.
        for i, end in enumerate([176, 192, 150, 184, 132]):
            y = 112 + i * 22
            c.soft_line([(60, y), (end, y)], 7.5, (255, 255, 255, 255), 0.20)
    else:
        for i, end in enumerate([0.55, 0.70] if size >= 24 else []):
            y = c.snap(y0 + (y1 - y0) * (0.55 + i * 0.16)) + c.px / 2
            c.soft_line([(x0 + 3 * c.px, y), (x0 + (x1 - x0) * end, y)], c.px, (255, 255, 255, 255), 0.30)

    corner = (x1 - f, y0 + f)
    flap = [(x1 - f, y0), (x1, y0 + f), corner]
    fm = c.mask(flap)
    c.shadow(fm, -4, 5, 5 if big else 1.5, 0.45 if big else 0.35, color=(10, 8, 50, 255), clip=tile)
    c.gradient(fm, hexrgb("#fffdf8"), hexrgb("#e2dccd"), angle=45)
    if big:
        c.line([(x1 - f, y0), (x1, y0 + f)], 1.6, hexrgb("#ffffff"))
    return c.done()


# III. Spectrum: five sheets in the app's mode colours, fanned behind one white page.
MODES = ["#e4573d", "#1f9e6e", "#7a4fe0", "#f0a81c", "#2a6fdb"]  # present, study, write, annotate, read


def spectrum(size):
    c = Canvas(size)
    big, small = size >= 48, size <= 24
    page = (44, 32, 154, 190) if size >= 48 else (34, 40, 128, 206)
    pivot = (74, page[3])
    sheets = list(zip([30, 24, 18, 12, 6], MODES)) if size >= 48 else         (list(zip([40, 32, 24, 16, 8], MODES)) if not small else [(42, "#7a4fe0"), (28, "#f0a81c"), (14, "#2a6fdb")])
    r = max(9, c.px * 1.2)
    outline = rrect(*page, r)
    if big:
        union = c.mask(outline)
        for a, _ in sheets:
            union = ImageChops.lighter(union, c.mask(rotate(outline, a, pivot)))
        c.shadow(union, 0, 5, 7, 0.28)
    for a, col in sheets:
        poly = rotate(outline, a, pivot)
        m = c.mask(poly)
        if big:
            c.shadow(m, -2, 2, 3, 0.18)
        c.fill(m, hexrgb(col))
        if not small:
            # A darker edge, the sheet's thickness.
            c.line(poly + poly[:1], max(1.2, c.px * 0.7), tuple(int(v * 0.78) for v in hexrgb(col)[:3]) + (255,))
    pm = c.mask(outline)
    if big:
        c.shadow(pm, -2, 2, 3, 0.20)
    c.fill(pm, hexrgb("#fbfaf6"))
    c.line(outline + outline[:1], max(1.3, c.px * (0.9 if size <= 32 else 0.6)), hexrgb("#c6c2b9"))
    lines = [(0.20, 0.78), (0.31, 0.86), (0.42, 0.66), (0.53, 0.80), (0.64, 0.58)] if big else \
        ([(0.26, 0.74), (0.46, 0.62)] if not small else [(0.30, 0.70)])
    w, h = page[2] - page[0], page[3] - page[1]
    for top, length in lines:
        y = page[1] + h * top
        if not big:
            y = c.snap(y) + c.px / 2
        c.line([(page[0] + w * 0.16, y), (page[0] + w * length, y)], 6 if big else c.px, hexrgb("#bcc1c9"))
    return c.done()


DESIGNS = [
    ("spread", "Spread", "An open notebook kept at its place by a vermilion ribbon", spread),
    ("dogear", "Dog-ear", "One ultramarine page, its corner turned down", dogear),
    ("spectrum", "Spectrum", "The five mode colours fanned behind a page", spectrum),
]


def font(name, px):
    try:
        return ImageFont.truetype(str(Path(os.environ.get("CANVAS_FONTS", "")) / name), px)
    except OSError:
        return ImageFont.load_default(px)


def sheet(sets, out):
    W, H = 2400, 1560
    bg, grey, rule = (244, 242, 236, 255), (96, 100, 110, 255), (214, 210, 200, 255)
    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)
    title, name, label, small = (font("Jura-Light.ttf", 40), font("Jura-Medium.ttf", 34),
                                 font("DMMono-Regular.ttf", 19), font("DMMono-Regular.ttf", 17))
    d.text((120, 92), "T H R E E   P R O P O S A L S", font=title, fill=(36, 42, 52, 255))
    d.text((122, 150), "EXCELLENTPDF MARK; EACH SIZE DRAWN FOR ITS OWN PIXEL GRID", font=small, fill=grey)
    col_w = (W - 240) // 3
    for i, (key, nice, line, _) in enumerate(DESIGNS):
        x = 120 + i * col_w
        icons = sets[key]
        if i:
            d.line([(x - 30, 230), (x - 30, H - 110)], fill=rule, width=1)
        d.text((x, 236), f"{['I', 'II', 'III'][i]}.  {nice}", font=name, fill=(36, 42, 52, 255))
        d.text((x, 284), line, font=small, fill=grey)
        img.alpha_composite(icons[1024].resize((440, 440), Image.LANCZOS), (x + 60, 330))
        # True sizes on paper.
        y, cx = 900, x
        for sz in [128, 64, 48, 32, 24, 16]:
            img.alpha_composite(icons[sz], (cx, y - sz))
            d.text((cx, y + 14), str(sz), font=label, fill=grey)
            cx += sz + 30
        # On a dark taskbar.
        d.rounded_rectangle([x, 966, x + col_w - 70, 1056], radius=10, fill=(32, 32, 34, 255))
        cx = x + 30
        for sz in [48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (cx, 1011 - sz // 2))
            cx += sz + 42
        d.text((x, 1068), "DARK TASKBAR", font=small, fill=grey)
        # Magnified grids.
        cx = x
        for sz, mag in [(16, 14), (32, 8)]:
            y = 1120
            d.rectangle([cx, y, cx + sz * mag, y + sz * mag], fill=(255, 255, 255, 255))
            img.alpha_composite(icons[sz].resize((sz * mag, sz * mag), Image.NEAREST), (cx, y))
            for j in range(sz + 1):
                d.line([(cx + j * mag, y), (cx + j * mag, y + sz * mag)], fill=(226, 223, 215, 255))
                d.line([(cx, y + j * mag), (cx + sz * mag, y + j * mag)], fill=(226, 223, 215, 255))
            d.text((cx, y + 256 + 14), f"{sz} PX  x{mag}", font=small, fill=grey)
            cx += sz * mag + 40
    img.convert("RGB").save(out, optimize=True)


def main():
    sets = {}
    for key, _, _, draw in DESIGNS:
        icons = {sz: draw(sz) for sz in sorted(set(PNG_SIZES) | set(ICO_SIZES))}
        sets[key] = icons
        folder = HERE / key / "png"
        folder.mkdir(parents=True, exist_ok=True)
        for sz in PNG_SIZES:
            icons[sz].save(folder / f"{key}-{sz}.png", optimize=True)
        icons[256].save(HERE / key / f"{key}.ico", format="ICO", sizes=[(s, s) for s in ICO_SIZES],
                        append_images=[icons[s] for s in ICO_SIZES[:-1]])
    sheet(sets, HERE / "comparison-sheet.png")
    print("wrote", HERE / "comparison-sheet.png")


if __name__ == "__main__":
    main()
