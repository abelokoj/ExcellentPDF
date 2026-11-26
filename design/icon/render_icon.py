"""Draws the ExcellentPDF app icon ("Nib & Leaf", Classic Indigo — Sharp Inscribed Edition,
NKJV Revelation 22:13; see philosophy.md) at every icon size, each size tuned for its own pixel
grid rather than shrunk, and writes the PNGs, the Windows .ico and the specimen sheet.

    python design/icon/render_icon.py

Needs Pillow and NumPy. Geometry is in units of a 256 square; each size is analytically rasterized
with supersampling and premultiplied box/Lanczos downsampling.
"""

from __future__ import annotations

import io
import math
import struct
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WIN_FONTS = Path(r"C:\Windows\Fonts")

# Sizes the .ico carries (Windows picks per DPI), and the PNGs kept for the other platforms.
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
PNG_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024]

SERIF_CANDIDATES = ["palai.ttf", "georgiai.ttf", "Palatino.ttc", "LiberationSerif-Italic.ttf"]


# ---------------------------------------------------------------------------
# Analytic SDF & compositing primitives (canonical 0..256 coordinate space)
# ---------------------------------------------------------------------------

def make_grid(px: int) -> tuple[np.ndarray, np.ndarray]:
    c = (np.arange(px, dtype=np.float32) + 0.5) * (256.0 / px)
    return np.meshgrid(c, c)


def aa_sharp(sdf: np.ndarray, px: int, sharpness: float = 0.68) -> np.ndarray:
    """Tight analytic anti-aliasing transition for razor-crisp edges."""
    w = (256.0 / px) * sharpness
    return np.clip(0.5 - sdf / w, 0.0, 1.0)


def sdf_rrect(
    x: np.ndarray, y: np.ndarray,
    x0: float, y0: float, x1: float, y1: float, r: float,
) -> np.ndarray:
    cx, cy = 0.5 * (x0 + x1), 0.5 * (y0 + y1)
    hx, hy = 0.5 * (x1 - x0) - r, 0.5 * (y1 - y0) - r
    qx = np.abs(x - cx) - hx
    qy = np.abs(y - cy) - hy
    return np.hypot(np.maximum(qx, 0.0), np.maximum(qy, 0.0)) + np.minimum(np.maximum(qx, qy), 0.0) - r


def sdf_rot_rrect(
    x: np.ndarray, y: np.ndarray,
    cx: float, cy: float, w: float, h: float, r: float, angle_deg: float,
) -> np.ndarray:
    rad = math.radians(angle_deg)
    ca, sa = math.cos(rad), math.sin(rad)
    dx, dy = x - cx, y - cy
    return sdf_rrect(dx * ca + dy * sa, -dx * sa + dy * ca, -w * 0.5, -h * 0.5, w * 0.5, h * 0.5, r)


def sdf_segment(x: np.ndarray, y: np.ndarray, ax: float, ay: float, bx: float, by: float) -> np.ndarray:
    pax, pay = x - ax, y - ay
    bax, bay = bx - ax, by - ay
    h = np.clip((pax * bax + pay * bay) / (bax * bax + bay * bay + 1e-9), 0.0, 1.0)
    return np.hypot(pax - bax * h, pay - bay * h)


def sdf_polygon(x: np.ndarray, y: np.ndarray, verts: list[tuple[float, float]]) -> np.ndarray:
    n = len(verts)
    d2 = np.full_like(x, 1e12, dtype=np.float32)
    inside = np.zeros(x.shape, dtype=bool)
    for i in range(n):
        x1, y1 = verts[i]
        x2, y2 = verts[(i + 1) % n]
        ex, ey = x2 - x1, y2 - y1
        wx, wy = x - x1, y - y1
        t = np.clip((wx * ex + wy * ey) / (ex * ex + ey * ey + 1e-9), 0.0, 1.0)
        bx, by = wx - ex * t, wy - ey * t
        d2 = np.minimum(d2, bx * bx + by * by)
        cond = ((y1 > y) != (y2 > y)) & (x < (x2 - x1) * (y - y1) / (y2 - y1 + 1e-9) + x1)
        inside ^= cond
    d = np.sqrt(d2)
    return np.where(inside, -d, d)


def sdf_halfplane(x: np.ndarray, y: np.ndarray, px: float, py: float, nx: float, ny: float) -> np.ndarray:
    return ((x - px) * nx + (y - py) * ny) / math.hypot(nx, ny)


def over(dst: np.ndarray, rgb: np.ndarray | tuple[float, float, float], alpha: np.ndarray) -> np.ndarray:
    a = np.clip(alpha, 0.0, 1.0)[..., None]
    src_rgb = (np.array(rgb, dtype=np.float32)[None, None, :] / 255.0) if isinstance(rgb, tuple) else rgb
    dst_rgb, dst_a = dst[..., :3], dst[..., 3:4]
    out_a = a + dst_a * (1.0 - a)
    safe_a = np.maximum(out_a, 1e-6)
    out = np.empty_like(dst)
    out[..., :3] = np.where(out_a > 1e-6, (src_rgb * a + dst_rgb * dst_a * (1.0 - a)) / safe_a, 0.0)
    out[..., 3:4] = out_a
    return out


def lin_grad(
    x: np.ndarray, y: np.ndarray,
    p0: tuple[float, float], p1: tuple[float, float],
    c0: tuple[float, float, float], c1: tuple[float, float, float],
) -> np.ndarray:
    dx, dy = p1[0] - p0[0], p1[1] - p0[1]
    t = np.clip(((x - p0[0]) * dx + (y - p0[1]) * dy) / (dx * dx + dy * dy + 1e-9), 0.0, 1.0)[..., None]
    a = np.array(c0, dtype=np.float32) / 255.0
    b = np.array(c1, dtype=np.float32) / 255.0
    return a * (1.0 - t) + b * t


def add_shadow(
    dst: np.ndarray, mask: np.ndarray, px: int,
    dx_u: float, dy_u: float, blur_u: float, opacity: float,
    color: tuple[float, float, float],
    clip_mask: np.ndarray | None = None,
) -> np.ndarray:
    scale = px / 256.0
    m_img = Image.fromarray((np.clip(mask, 0.0, 1.0) * 255.0).astype(np.uint8), mode="L")
    shifted = Image.new("L", (px, px), 0)
    shifted.paste(m_img, (round(dx_u * scale), round(dy_u * scale)))
    if blur_u > 0:
        shifted = shifted.filter(ImageFilter.GaussianBlur(radius=blur_u * scale))
    sh = np.asarray(shifted, dtype=np.float32) / 255.0 * opacity
    if clip_mask is not None:
        sh *= clip_mask
    return over(dst, color, sh)


def to_pil(rgba: np.ndarray) -> Image.Image:
    return Image.fromarray(np.clip(np.round(rgba * 255.0), 0, 255).astype(np.uint8), mode="RGBA")


def downsample(rgba: np.ndarray, target: int) -> Image.Image:
    img = to_pil(rgba)
    if img.size[0] == target:
        return img
    arr = rgba.copy()
    arr[..., :3] *= arr[..., 3:4]
    pm = to_pil(arr)
    factor = img.size[0] // target
    small_pm = pm.reduce(factor) if (factor >= 2 and img.size[0] == target * factor) else pm.resize((target, target), Image.LANCZOS)
    s = np.asarray(small_pm, dtype=np.float32) / 255.0
    a = s[..., 3:4]
    out = np.zeros_like(s)
    out[..., :3] = np.where(a > 1e-4, np.clip(s[..., :3] / np.maximum(a, 1e-4), 0.0, 1.0), 0.0)
    out[..., 3:4] = a
    return to_pil(out)


def load_font(candidates: list[str], px: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for name in candidates:
        for base in (WIN_FONTS, Path("/usr/share/fonts/truetype/liberation"), Path("/System/Library/Fonts/Supplemental")):
            p = base / name
            if p.exists():
                try:
                    return ImageFont.truetype(str(p), px)
                except OSError:
                    pass
    return ImageFont.load_default(px)


def sample_cubic(
    c0: tuple[float, float], c1: tuple[float, float],
    c2: tuple[float, float], c3: tuple[float, float],
    n_seg: int, scale: float = 1.0,
) -> list[tuple[float, float]]:
    pts = []
    for k in range(n_seg + 1):
        t = k / n_seg
        mt = 1.0 - t
        pts.append((
            (mt**3 * c0[0] + 3 * mt**2 * t * c1[0] + 3 * mt * t**2 * c2[0] + t**3 * c3[0]) * scale,
            (mt**3 * c0[1] + 3 * mt**2 * t * c1[1] + 3 * mt * t**2 * c2[1] + t**3 * c3[1]) * scale,
        ))
    return pts


def draw_text_with_superscript(
    td: ImageDraw.ImageDraw,
    x_px: float,
    baseline_px: float,
    open_quote: str,
    verse_num: str,
    rest_text: str,
    main_font,
    sup_font,
    fill: int,
) -> float:
    """Draws opening quote + true typographic superscript verse number + thin space + text."""
    cx = x_px
    if open_quote:
        td.text((cx, baseline_px), open_quote, font=main_font, fill=fill, anchor="ls")
        cx += td.textlength(open_quote, font=main_font)
    if verse_num:
        bbox = td.textbbox((0, 0), "H", font=main_font)
        cap_h = max(1, bbox[3] - bbox[1])
        td.text((cx, baseline_px - cap_h * 0.36), verse_num, font=sup_font, fill=fill, anchor="ls")
        cx += td.textlength(verse_num, font=sup_font) + td.textlength(" ", font=sup_font) * 0.45
    if rest_text:
        td.text((cx, baseline_px), rest_text, font=main_font, fill=fill, anchor="ls")
        cx += td.textlength(rest_text, font=main_font)
    return cx - x_px


def measure_with_superscript(
    probe: ImageDraw.ImageDraw,
    open_quote: str,
    verse_num: str,
    rest_text: str,
    main_font,
    sup_font,
) -> float:
    w = 0.0
    if open_quote:
        w += probe.textlength(open_quote, font=main_font)
    if verse_num:
        w += probe.textlength(verse_num, font=sup_font) + probe.textlength(" ", font=sup_font) * 0.45
    if rest_text:
        w += probe.textlength(rest_text, font=main_font)
    return w


# ---------------------------------------------------------------------------
# Icon Renderer: "Nib & Leaf" (Classic Indigo — Sharp Inscribed, Rev 22:13)
# ---------------------------------------------------------------------------

def draw(size: int) -> Image.Image:
    # Always supersample >= 2x up through 512 px for razor-sharp serifs, rules and nib ridges
    ss = 8 if size <= 32 else (4 if size <= 128 else (2 if size <= 512 else 1))
    px = size * ss
    scale = px / 256.0
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Crisp cool-periwinkle back sheet (tabs / workspaces / notebook stack) — omitted at <=20 px
    if not small:
        d_back = sdf_rot_rrect(x, y, 105.0, 122.0, 144.0, 192.0, 8.5, -5.2)
        m_back = aa_sharp(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -0.5, 2.8, 3.6, 0.18, (14, 20, 46))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (224, 236, 255), (182, 202, 246))
        canvas = over(canvas, back_col, m_back)
        be_w = 2.8 if medium else 2.0
        m_be = aa_sharp(np.maximum(d_back, -(d_back + be_w)), px)
        canvas = over(canvas, (94, 120, 186), m_be * 0.85)

    # 2. Hero crisp warm-ivory folded page
    if small:
        px0, py0, px1, py1, pr, fold = 20.0, 14.0, 192.0, 238.0, 10.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 32.0, 20.0, 194.0, 234.0, 8.5, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 42.0, 24.0, 196.0, 228.0, 8.0, 36.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_sharp(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 0.8, 3.5, 4.2, 0.28, (12, 18, 44))

    paper = lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 252), (245, 243, 236))
    canvas = over(canvas, paper, m_page)

    # 3. Visible ruled stationery lines & NKJV Revelation 22:13 in Palatino Italic
    verse_num = "13"
    lines = [
        ("I am the Alpha and", 128.0),
        ("the Omega, the Beginning", 128.0),
        ("and the End, the First", 108.0),
        ("and the Last.\u201d", 94.0),
    ]
    attribution = "\u2014 (NKJV Revelation 22:13)"
    line_ys = [52.0, 67.5, 83.0, 98.5, 114.0, 129.5, 145.0, 160.5]

    if not small:
        margin_l, margin_r = px0 + 12.0, px1 - 12.0
        if size >= 40:
            d_rules = np.full_like(x, 1e6, dtype=np.float32)
            rule_h = max(1.12, 1.0 * (256.0 / size))
            for ly_rel in line_ys:
                ly = py0 + ly_rel
                if ly > py1 - 14.0:
                    continue
                r_end = (px1 - fold - 8.0) if ly < py0 + fold + 4.0 else margin_r
                d_r = sdf_rrect(x, y, margin_l, ly - rule_h * 0.5, r_end, ly + rule_h * 0.5, 0.15)
                d_rules = np.minimum(d_rules, d_r)
            canvas = over(canvas, (108, 134, 188), aa_sharp(d_rules, px, 0.72) * m_page * 0.58)

        if size <= 48:
            # Crisp simplified typographic bars for small icon grids (never mushy micro-text)
            d_l1 = sdf_rrect(x, y, px0 + 15.0, py0 + 40.0, px1 - 24.0, py0 + 48.5, 2.2)
            d_l2 = sdf_rrect(x, y, px0 + 15.0, py0 + 58.0, px1 - 28.0, py0 + 66.5, 2.2)
            d_l3 = sdf_rrect(x, y, px0 + 15.0, py0 + 76.0, px1 - 52.0, py0 + 84.5, 2.2)
            canvas = over(canvas, (92, 106, 138), aa_sharp(d_l1, px) * m_page * 0.96)
            canvas = over(canvas, (102, 116, 146), aa_sharp(d_l2, px) * m_page * 0.92)
            canvas = over(canvas, (124, 136, 162), aa_sharp(d_l3, px) * m_page * 0.88)
        else:
            page_w = px1 - px0
            text_x = px0 + 12.0
            font_px = max(7, round(page_w * 0.076 * scale))
            probe = ImageDraw.Draw(Image.new("L", (4, 4), 0))

            while font_px > 6:
                font = load_font(SERIF_CANDIDATES, font_px)
                sup_font = load_font(SERIF_CANDIDATES, max(5, round(font_px * 0.62)))
                fits = True
                for i, (txt, max_u) in enumerate(lines):
                    w_px = (
                        measure_with_superscript(probe, "\u201c", verse_num, txt, font, sup_font)
                        if i == 0
                        else probe.textlength(txt, font=font)
                    )
                    if w_px > max_u * scale:
                        fits = False
                        break
                if fits:
                    break
                font_px -= 1

            font = load_font(SERIF_CANDIDATES, font_px)
            sup_font = load_font(SERIF_CANDIDATES, max(5, round(font_px * 0.62)))

            txt_mask = Image.new("L", (px, px), 0)
            td = ImageDraw.Draw(txt_mask)
            for i, (txt, _max_u) in enumerate(lines):
                base_px = (py0 + line_ys[i] - 2.3) * scale
                if i == 0:
                    draw_text_with_superscript(td, text_x * scale, base_px, "\u201c", verse_num, txt, font, sup_font, 255)
                else:
                    td.text((text_x * scale, base_px), txt, font=font, fill=252, anchor="ls")

            attr_px = max(6, round(font_px * 0.76))
            attr_font = load_font(SERIF_CANDIDATES, attr_px)
            while attr_px > 5:
                if probe.textlength(attribution, font=attr_font) <= 90.0 * scale:
                    break
                attr_px -= 1
                attr_font = load_font(SERIF_CANDIDATES, attr_px)
            td.text((text_x * scale, (py0 + line_ys[len(lines)] - 2.3) * scale), attribution, font=attr_font, fill=214, anchor="ls")

            raw_a = np.asarray(txt_mask, dtype=np.float32) / 255.0
            crisp_a = np.clip(raw_a ** 0.84, 0.0, 1.0) * m_page
            canvas = over(canvas, (22, 30, 56), crisp_a)

    # Crisp high-contrast page edge border
    edge_w = 9.5 if small else (5.2 if medium else 2.4)
    edge_col = (58, 72, 114) if (small or medium) else (92, 108, 146)
    m_edge = aa_sharp(np.maximum(d_page, -(d_page + edge_w)), px)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.98 if medium else 0.92)))

    # Crisp folded top-right corner
    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 2.0))
    m_flap = aa_sharp(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -1.6, 2.2, 2.4, 0.24, (16, 24, 54), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (222, 226, 236), (190, 198, 218)), m_flap)
    canvas = over(canvas, edge_col, aa_sharp(np.maximum(d_flap, -(d_flap + edge_w * 0.85)), px) * m_page * 0.88)

    # 4. Crisp vivid calligraphic ink flourish
    tip_x, tip_y = (126.0, 176.0) if small else (142.0, 177.0)
    p0 = (46.0, 204.0) if small else (58.0, 200.0)
    p1, p2, p3 = (86.0, 210.0), (112.0, 183.0), (tip_x, tip_y)
    max_r = 11.2 if small else 7.2
    pts = []
    for i in range(56):
        t = i / 55.0
        mt = 1.0 - t
        bx = mt**3 * p0[0] + 3 * mt**2 * t * p1[0] + 3 * mt * t**2 * p2[0] + t**3 * p3[0]
        by = mt**3 * p0[1] + 3 * mt**2 * t * p1[1] + 3 * mt * t**2 * p2[1] + t**3 * p3[1]
        env = math.sin(math.pi * (0.03 + 0.91 * t)) ** 1.15
        pts.append((bx, by, max_r * max(0.12, env)))
    d_ink = np.full_like(x, 1e6, dtype=np.float32)
    for i in range(len(pts) - 1):
        ax, ay, ar = pts[i]
        bx_, by_, br = pts[i + 1]
        d_ink = np.minimum(d_ink, sdf_segment(x, y, ax, ay, bx_, by_) - 0.5 * (ar + br))
    canvas = over(canvas, lin_grad(x, y, p0, p3, (44, 68, 232), (88, 46, 216)), aa_sharp(d_ink, px) * m_page)

    # 5. Razor-sharp faceted Royal-Indigo Fountain Nib + Machined Dual-Groove Brass Ferrule
    nib_sc = 1.14 if small else 0.94
    ang = math.radians(-42.0)
    ca, sa = math.cos(ang), math.sin(ang)
    dx, dy = x - tip_x, y - tip_y
    u = dx * ca + dy * sa
    v = -dx * sa + dy * ca

    upper_a = sample_cubic((0.0, 0.0), (13.0, 3.8), (33.0, 25.5), (54.0, 32.0), 14, nib_sc)
    upper_b = sample_cubic((54.0, 32.0), (68.0, 34.0), (84.0, 23.5), (98.0, 18.8), 10, nib_sc)
    pos_flank = upper_a + upper_b[1:]
    nib_poly = pos_flank + [(pu, -pv) for (pu, pv) in reversed(pos_flank[1:])]

    base_u, collar_end, collar_hw = 96.0 * nib_sc, 120.5 * nib_sc, 21.5 * nib_sc
    collar_poly = [
        (base_u, collar_hw),
        (collar_end, collar_hw + 1.2 * nib_sc),
        (collar_end, -(collar_hw + 1.2 * nib_sc)),
        (base_u, -collar_hw),
    ]
    d_nib = sdf_polygon(u, v, nib_poly)
    d_collar = sdf_polygon(u, v, collar_poly) - 0.8 * nib_sc
    m_pen_all = aa_sharp(np.minimum(d_nib, d_collar), px)

    if not small:
        canvas = add_shadow(canvas, m_pen_all, px, 1.8, 4.8, 3.8, 0.34, (10, 14, 38))

    m_nib = aa_sharp(d_nib, px)
    left_facet = lin_grad(u, v, (0, -30 * nib_sc), (100 * nib_sc, 10 * nib_sc), (110, 102, 252), (48, 40, 182))
    right_facet = lin_grad(u, v, (0, 0), (100 * nib_sc, 32 * nib_sc), (36, 26, 138), (14, 10, 66))
    ridge = aa_sharp(v, px, 0.55)[..., None]
    canvas = over(canvas, left_facet * ridge + right_facet * (1.0 - ridge), m_nib)

    if not small:
        m_nib_rim = aa_sharp(np.maximum(d_nib, -(d_nib + 1.8 * nib_sc)), px) * (v < 0).astype(np.float32)
        canvas = over(canvas, (224, 232, 255), m_nib_rim * 0.65)
        m_nib_dark = aa_sharp(np.maximum(d_nib, -(d_nib + 1.5 * nib_sc)), px) * (v >= 0).astype(np.float32)
        canvas = over(canvas, (12, 10, 48), m_nib_dark * 0.55)
        d_ridge_glint = sdf_segment(u, v, 56.0 * nib_sc, -0.5 * nib_sc, base_u - 1.0 * nib_sc, -0.5 * nib_sc) - 0.7 * nib_sc
        canvas = over(canvas, (186, 194, 255), aa_sharp(d_ridge_glint, px) * m_nib * 0.36)

    m_collar = aa_sharp(d_collar, px)
    collar_rgb = lin_grad(u, v, (base_u + 10 * nib_sc, -24 * nib_sc), (base_u + 10 * nib_sc, 24 * nib_sc),
                          (255, 228, 122), (174, 108, 12))
    canvas = over(canvas, collar_rgb, m_collar)
    if not small and not medium:
        d_ring1 = np.abs(u - (base_u + 5.0 * nib_sc)) - 0.9 * nib_sc
        d_ring2 = np.abs(u - (collar_end - 4.0 * nib_sc)) - 0.75 * nib_sc
        canvas = over(canvas, (94, 52, 4), aa_sharp(np.minimum(d_ring1, d_ring2), px) * m_collar * 0.55)
        d_spec = np.abs(v + collar_hw * 0.46) - 2.2 * nib_sc
        canvas = over(canvas, (255, 253, 240), aa_sharp(d_spec, px) * m_collar * 0.46)
        m_c_edge = aa_sharp(np.maximum(d_collar, -(d_collar + 1.4 * nib_sc)), px)
        canvas = over(canvas, (118, 70, 10), m_c_edge * 0.48)

    hole_u = 49.0 * nib_sc
    hole_r = (8.8 if small else (7.2 if medium else 6.2)) * nib_sc
    slit_w = (2.6 if small else (1.95 if medium else 1.4)) * nib_sc
    d_slit = np.minimum(np.hypot(u - hole_u, v) - hole_r, sdf_segment(u, v, 8.0 * nib_sc, 0.0, hole_u, 0.0) - slit_w)
    slit_rgb = lin_grad(u, v, (10 * nib_sc, -8 * nib_sc), (58 * nib_sc, 8 * nib_sc), (255, 240, 160), (242, 174, 34))
    canvas = over(canvas, slit_rgb, aa_sharp(d_slit, px) * m_nib)

    if not small and not medium:
        d_tip = np.hypot(u - 3.4 * nib_sc, v) - 2.1 * nib_sc
        canvas = over(canvas, (255, 248, 214), aa_sharp(d_tip, px) * m_nib * 0.92)

    img = downsample(canvas, size)
    if size >= 128:
        rgb = img.convert("RGB").filter(ImageFilter.UnsharpMask(radius=0.8, percent=55, threshold=2))
        a = img.getchannel("A")
        img = Image.merge("RGBA", (*rgb.split(), a))
    return img


# ---------------------------------------------------------------------------
# Specimen Plate & Windows ICO Writer
# ---------------------------------------------------------------------------

def sheet(icons: dict[int, Image.Image], out: Path) -> None:
    """Specimen plate: the mark large, every shipped size on light & dark taskbars, and magnified pixel grids."""
    W, H = 2400, 1500
    bg, ink_grey, rule = (244, 242, 236, 255), (92, 98, 108, 255), (206, 203, 194, 255)
    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)
    title = load_font(["segoeuib.ttf", "segoeui.ttf"], 36)
    label = load_font(["consola.ttf"], 20)
    small = load_font(["consola.ttf"], 17)

    big = icons[1024].resize((880, 880), Image.LANCZOS)
    img.alpha_composite(big, (110, 250))
    d.text((140, 112), "N I B  &  L E A F", font=title, fill=(30, 36, 54, 255))
    d.text((140, 166), "PLATE II: THE EXCELLENTPDF MARK (CLASSIC INDIGO — REVELATION 22:13)", font=small, fill=ink_grey)
    d.line([(1080, 140), (1080, H - 140)], fill=rule, width=1)

    x, base = 1160, 560
    for sz in [256, 128, 64, 48, 32, 24, 16]:
        img.alpha_composite(icons[sz], (x, base - sz))
        d.text((x, base + 22), f"{sz}", font=label, fill=ink_grey)
        x += sz + 46

    dark = (1160, 650, 1160 + 40 + sum(s + 64 for s in [64, 48, 40, 32, 24, 20, 16]) - 24, 790)
    d.rounded_rectangle(dark, radius=10, fill=(32, 32, 34, 255))
    x = 1200
    for sz in [64, 48, 40, 32, 24, 20, 16]:
        img.alpha_composite(icons[sz], (x, dark[3] - 30 - sz))
        x += sz + 64
    d.text((1160, 806), "ON A DARK TASKBAR (#202022)", font=small, fill=ink_grey)

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

    d.text((140, H - 108), "page: NKJV Revelation 22:13  /  nib: royal indigo & machined satin brass", font=small, fill=ink_grey)
    img.convert("RGB").save(out, optimize=True)


def encode_dib_32bpp(img: Image.Image) -> bytes:
    """Encodes an RGBA image as an uncompressed 32-bit BGRA BITMAPINFOHEADER DIB for Windows .ico."""
    w, h = img.size
    rgba = np.asarray(img.convert("RGBA"), dtype=np.uint8)
    # Bottom-up BGRA pixel array.
    bottom_up = rgba[::-1, :, :]
    bgra = bottom_up[..., [2, 1, 0, 3]].tobytes()
    # 1-bpp AND mask (bit=1 where transparent), padded per row to 32-bit DWORD boundary.
    row_bytes = ((w + 31) // 32) * 4
    and_mask = bytearray(row_bytes * h)
    alpha = bottom_up[..., 3]
    for y in range(h):
        for x in range(w):
            if alpha[y, x] == 0:
                and_mask[y * row_bytes + (x >> 3)] |= 0x80 >> (x & 7)
    header = struct.pack(
        "<IiiHHIIiiII",
        40,              # biSize
        w,               # biWidth
        h * 2,           # biHeight (XOR + AND)
        1,               # biPlanes
        32,              # biBitCount
        0,               # biCompression (BI_RGB)
        len(bgra),       # biSizeImage
        0, 0, 0, 0,
    )
    return header + bgra + bytes(and_mask)


def write_ico(icons: dict[int, Image.Image], out: Path) -> None:
    """Writes a hybrid BMP-DIB (<256 px) + PNG (256 px) Windows .ico carrying every size in ICO_SIZES."""
    entries: list[tuple[int, bytes]] = []
    for sz in ICO_SIZES:
        if sz >= 256:
            buf = io.BytesIO()
            icons[sz].save(buf, format="PNG", optimize=True)
            entries.append((sz, buf.getvalue()))
        else:
            entries.append((sz, encode_dib_32bpp(icons[sz])))

    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory = bytearray()
    payload = bytearray()
    for sz, data in entries:
        dim = 0 if sz >= 256 else sz
        directory.extend(struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset))
        payload.extend(data)
        offset += len(data)
    out.write_bytes(header + directory + payload)


def main() -> None:
    icons = {sz: draw(sz) for sz in sorted(set(PNG_SIZES) | set(ICO_SIZES))}
    png_dir = HERE / "png"
    png_dir.mkdir(exist_ok=True)
    for sz in PNG_SIZES:
        icons[sz].save(png_dir / f"excellentpdf-{sz}.png", optimize=True)
    ico = ROOT / "packaging" / "windows" / "excellentpdf.ico"
    write_ico(icons, ico)
    app_asset = ROOT / "crates" / "excellent-app" / "assets" / "app-icon-64.png"
    app_asset.parent.mkdir(parents=True, exist_ok=True)
    icons[64].save(app_asset, format="PNG", optimize=True)
    sheet(icons, HERE / "excellentpdf-icon-sheet.png")
    print(f"wrote {ico} ({ico.stat().st_size:,} bytes, {len(ICO_SIZES)} sizes) and {app_asset}")


if __name__ == "__main__":
    main()
