"""Generates 3 alternative app icon concepts for EcclesiaPDF and a side-by-side comparison specimen sheet.

    python design/icon/alternatives/render_alternatives.py

Does NOT modify the active icon (design/icon/render_icon.py or packaging/windows/ecclesiapdf.ico).
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Callable

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
ICON_DIR = HERE.parent
WIN_FONTS = Path(r"C:\Windows\Fonts")

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024]


# ---------------------------------------------------------------------------
# Math / SDF helpers (canonical 0..256 coordinate space)
# ---------------------------------------------------------------------------

def make_grid(px: int) -> tuple[np.ndarray, np.ndarray]:
    """Pixel centers mapped into [0, 256]."""
    c = (np.arange(px, dtype=np.float32) + 0.5) * (256.0 / px)
    return np.meshgrid(c, c)


def aa_mask(sdf: np.ndarray, px: int) -> np.ndarray:
    """Smooth coverage [0..1] from signed distance (negative inside)."""
    w = 256.0 / px
    return np.clip(0.5 - sdf / (w * 1.05), 0.0, 1.0)


def sdf_superellipse(
    x: np.ndarray,
    y: np.ndarray,
    cx: float,
    cy: float,
    rx: float,
    ry: float,
    n: float = 4.6,
) -> np.ndarray:
    """Approximate Euclidean signed distance to a superellipse (squircle)."""
    dx = np.abs(x - cx) / rx
    dy = np.abs(y - cy) / ry
    # Avoid 0^negative in gradient
    eps = 1e-7
    val = (dx + eps) ** n + (dy + eps) ** n
    f = val ** (1.0 / n) - 1.0
    # Gradient magnitude in world units for Euclidean normalization
    gx = (val ** (1.0 / n - 1.0)) * ((dx + eps) ** (n - 1.0)) / rx
    gy = (val ** (1.0 / n - 1.0)) * ((dy + eps) ** (n - 1.0)) / ry
    g = np.hypot(gx, gy) + 1e-6
    return f / g


def sdf_rrect(
    x: np.ndarray,
    y: np.ndarray,
    x0: float,
    y0: float,
    x1: float,
    y1: float,
    r: float,
) -> np.ndarray:
    """Exact signed distance to an axis-aligned rounded rectangle."""
    cx, cy = 0.5 * (x0 + x1), 0.5 * (y0 + y1)
    hx, hy = 0.5 * (x1 - x0) - r, 0.5 * (y1 - y0) - r
    qx = np.abs(x - cx) - hx
    qy = np.abs(y - cy) - hy
    outside = np.hypot(np.maximum(qx, 0.0), np.maximum(qy, 0.0))
    inside = np.minimum(np.maximum(qx, qy), 0.0)
    return outside + inside - r


def sdf_rot_rrect(
    x: np.ndarray,
    y: np.ndarray,
    cx: float,
    cy: float,
    w: float,
    h: float,
    r: float,
    angle_deg: float,
) -> np.ndarray:
    """Exact signed distance to a rotated rounded rectangle."""
    rad = math.radians(angle_deg)
    ca, sa = math.cos(rad), math.sin(rad)
    dx, dy = x - cx, y - cy
    lx = dx * ca + dy * sa
    ly = -dx * sa + dy * ca
    return sdf_rrect(lx, ly, -w * 0.5, -h * 0.5, w * 0.5, h * 0.5, r)


def sdf_segment(
    x: np.ndarray,
    y: np.ndarray,
    ax: float,
    ay: float,
    bx: float,
    by: float,
) -> np.ndarray:
    """Distance from (x, y) to line segment AB."""
    pax, pay = x - ax, y - ay
    bax, bay = bx - ax, by - ay
    denom = bax * bax + bay * bay + 1e-9
    h = np.clip((pax * bax + pay * bay) / denom, 0.0, 1.0)
    return np.hypot(pax - bax * h, pay - bay * h)


def sdf_polygon(x: np.ndarray, y: np.ndarray, verts: list[tuple[float, float]]) -> np.ndarray:
    """Exact signed distance to a simple polygon (negative inside, CW or CCW)."""
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
    """Signed distance to half-plane: negative where dot((p - P), n) < 0."""
    mag = math.hypot(nx, ny)
    return ((x - px) * nx + (y - py) * ny) / mag


def over(dst: np.ndarray, rgb: np.ndarray | tuple[float, float, float], alpha: np.ndarray) -> np.ndarray:
    """Porter-Duff source-over compositing on float32 RGBA [0..1] arrays."""
    a = np.clip(alpha, 0.0, 1.0)[..., None]
    if isinstance(rgb, tuple):
        src_rgb = np.array(rgb, dtype=np.float32)[None, None, :] / 255.0
    else:
        src_rgb = rgb
    dst_rgb = dst[..., :3]
    dst_a = dst[..., 3:4]
    out_a = a + dst_a * (1.0 - a)
    safe_a = np.maximum(out_a, 1e-6)
    out_rgb = (src_rgb * a + dst_rgb * dst_a * (1.0 - a)) / safe_a
    out = np.empty_like(dst)
    out[..., :3] = np.where(out_a > 1e-6, out_rgb, 0.0)
    out[..., 3:4] = out_a
    return out


def lin_grad(
    x: np.ndarray,
    y: np.ndarray,
    p0: tuple[float, float],
    p1: tuple[float, float],
    c0: tuple[float, float, float],
    c1: tuple[float, float, float],
) -> np.ndarray:
    """Linear RGB gradient [0..1] between p0 and p1."""
    dx, dy = p1[0] - p0[0], p1[1] - p0[1]
    t = np.clip(((x - p0[0]) * dx + (y - p0[1]) * dy) / (dx * dx + dy * dy + 1e-9), 0.0, 1.0)[..., None]
    a = np.array(c0, dtype=np.float32) / 255.0
    b = np.array(c1, dtype=np.float32) / 255.0
    return a * (1.0 - t) + b * t


def add_shadow(
    dst: np.ndarray,
    mask: np.ndarray,
    px: int,
    dx_u: float,
    dy_u: float,
    blur_u: float,
    opacity: float,
    color: tuple[float, float, float] = (10, 18, 40),
    clip_mask: np.ndarray | None = None,
) -> np.ndarray:
    """Blurred drop shadow from an analytic [0..1] coverage mask."""
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
    arr = np.clip(np.round(rgba * 255.0), 0, 255).astype(np.uint8)
    return Image.fromarray(arr, mode="RGBA")


def downsample(rgba: np.ndarray, target: int) -> Image.Image:
    """High-quality box/Lanczos downsample from supersampled buffer."""
    img = to_pil(rgba)
    if img.size[0] == target:
        return img
    # Premultiply alpha before resizing to avoid dark/bright fringes
    arr = rgba.copy()
    arr[..., :3] *= arr[..., 3:4]
    pm = to_pil(arr)
    factor = img.size[0] // target
    if factor >= 2 and img.size[0] == target * factor:
        small_pm = pm.reduce(factor)
    else:
        small_pm = pm.resize((target, target), Image.LANCZOS)
    s = np.asarray(small_pm, dtype=np.float32) / 255.0
    a = s[..., 3:4]
    out = np.zeros_like(s)
    out[..., :3] = np.where(a > 1e-4, np.clip(s[..., :3] / np.maximum(a, 1e-4), 0.0, 1.0), 0.0)
    out[..., 3:4] = a
    return to_pil(out)


def render_ss(size: int) -> int:
    if size <= 32:
        return 8
    if size <= 128:
        return 4
    if size <= 256:
        return 2
    return 1


# ===========================================================================
# OPTION 1: "FOLIO"
# Deep cobalt squircle + layered crisp paper + warm amber highlight sweep
# ===========================================================================

def draw_folio(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Cobalt squircle tile
    r_tile = 118.0 if small else (114.0 if medium else 110.0)
    d_tile = sdf_superellipse(x, y, 128.0, 128.0, r_tile, r_tile, n=5.0)
    m_tile = aa_mask(d_tile, px)

    if not small and not medium:
        canvas = add_shadow(canvas, m_tile, px, 0.0, 5.0, 8.0, 0.28, (8, 18, 48))

    # Rich royal-cobalt gradient with gentle top-left studio light
    bg = lin_grad(x, y, (32, 20), (224, 240), (42, 102, 232), (16, 40, 112))
    glow = np.exp(-(((x - 86.0) / 115.0) ** 2 + ((y - 54.0) / 95.0) ** 2))[..., None]
    bg = np.clip(bg + glow * np.array([0.07, 0.11, 0.14], dtype=np.float32), 0.0, 1.0)
    canvas = over(canvas, bg, m_tile)

    # Crisp inner specular top rim + dark bottom rim (reads sharply on light & dark taskbars)
    rim_w = 5.0 if small else (3.6 if medium else 2.4)
    m_rim = aa_mask(np.maximum(d_tile, -(d_tile + rim_w)), px)
    rim_t = np.clip((256.0 - y) / 220.0, 0.0, 1.0)
    canvas = over(canvas, (190, 218, 255), m_rim * (0.14 + 0.28 * rim_t))

    # 2. Subtle fanned back sheet (tabs / workspaces depth) — neatly tucked behind bottom-left
    if not small:
        d_back = sdf_rot_rrect(x, y, 118.0, 126.0, 110.0, 146.0, 13.0, -7.0)
        m_back = aa_mask(d_back, px) * m_tile
        back_col = lin_grad(x, y, (52, 50), (165, 200), (186, 214, 255), (112, 154, 240))
        canvas = over(canvas, back_col, m_back * 0.38)

    # 3. Hero white folded page
    if small:
        px0, py0, px1, py1, pr, fold = 56.0, 38.0, 204.0, 218.0, 16.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 62.0, 42.0, 198.0, 214.0, 14.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 68.0, 46.0, 196.0, 212.0, 13.0, 38.0

    # Cut top-right diagonal for fold
    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page_rect = sdf_rrect(x, y, px0, py0, px1, py1, pr)
    d_page = np.maximum(d_page_rect, d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 0.0, 7.0, 10.0, 0.36, (8, 18, 56), clip_mask=m_tile)

    paper_rgb = lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 253), (240, 239, 233))
    canvas = over(canvas, paper_rgb, m_page)

    # 4. Page content: luminous amber-gold highlight sweep + crisp text lines
    if small:
        # Bold, unmistakable warm amber highlight bar + dark ink line
        d_hl = sdf_rrect(x, y, px0 + 18.0, 114.0, px1 - 18.0, 152.0, 10.0)
        hl_col = lin_grad(x, y, (px0, 114), (px1, 152), (253, 205, 58), (242, 152, 18))
        canvas = over(canvas, hl_col, aa_mask(d_hl, px) * m_page)
        d_t1 = sdf_rrect(x, y, px0 + 22.0, 76.0, px1 - 54.0, 92.0, 6.0)
        canvas = over(canvas, (152, 166, 190), aa_mask(d_t1, px) * m_page)
        d_t2 = sdf_rrect(x, y, px0 + 22.0, 174.0, px1 - 36.0, 190.0, 6.0)
        canvas = over(canvas, (152, 166, 190), aa_mask(d_t2, px) * m_page)
    else:
        # Quiet line above (clear of fold)
        d_t0 = sdf_rrect(x, y, px0 + 20.0, py0 + 36.0, px1 - fold - 10.0, py0 + 45.0, 4.5)
        canvas = over(canvas, (176, 186, 204), aa_mask(d_t0, px) * m_page)

        # Warm sunlit amber-gold highlight band (Annotate mode!)
        hy0, hy1 = 112.0, 140.0
        d_hl = sdf_rrect(x, y, px0 + 14.0, hy0, px1 - 14.0, hy1, 7.5)
        hl_col = lin_grad(x, y, (px0 + 14, hy0), (px1 - 14, hy1), (253, 211, 68), (245, 158, 11))
        canvas = over(canvas, hl_col, aa_mask(d_hl, px) * m_page)

        # Crisp dark-ink text line sitting inside the highlight (proves readability!)
        d_hi_txt = sdf_rrect(x, y, px0 + 22.0, hy0 + 9.5, px1 - 26.0, hy1 - 9.5, 4.5)
        canvas = over(canvas, (28, 39, 64), aa_mask(d_hi_txt, px) * m_page)

        # Two calm lines below with ragged right edge
        d_t1 = sdf_rrect(x, y, px0 + 20.0, 156.0, px1 - 24.0, 165.0, 4.5)
        d_t2 = sdf_rrect(x, y, px0 + 20.0, 176.0, px1 - 46.0, 185.0, 4.5)
        canvas = over(canvas, (176, 186, 204), aa_mask(d_t1, px) * m_page)
        canvas = over(canvas, (190, 198, 212), aa_mask(d_t2, px) * m_page)

    # 5. Sculpted dog-ear corner fold with diagonal crease shadow
    flap_verts = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = sdf_polygon(x, y, flap_verts)
    # Round the inner bottom-left corner of the flap slightly via smooth intersection with rounded box
    d_flap_r = sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 5.5)
    d_flap_clean = np.maximum(d_flap, d_flap_r)
    m_flap = aa_mask(d_flap_clean, px) * m_page

    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.5, 4.0, 4.5, 0.24, (14, 28, 68), clip_mask=m_page)

    flap_col = lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (218, 228, 245), (186, 202, 232))
    canvas = over(canvas, flap_col, m_flap)

    return downsample(canvas, size)


# ===========================================================================
# OPTION 2: "NIB & LEAF"
# Unframed Fluent silhouette: layered pages + sculpted indigo/gold fountain nib
# ===========================================================================

def draw_nib_and_leaf(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Back sheet (tabs / notebook stack) — skipped at <=20px
    if not small:
        d_back = sdf_rot_rrect(x, y, 106.0, 122.0, 144.0, 192.0, 16.0, -5.5)
        m_back = aa_mask(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -1.0, 5.0, 8.0, 0.16, (18, 24, 52))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (222, 232, 252), (192, 208, 244))
        canvas = over(canvas, back_col, m_back)
        # Subtle border on back sheet
        m_back_edge = aa_mask(np.maximum(d_back, -(d_back + 2.2)), px)
        canvas = over(canvas, (132, 152, 202), m_back_edge * 0.55)

    # 2. Hero front page (crisp warm ivory sheet with folded corner)
    if small:
        px0, py0, px1, py1, pr, fold = 22.0, 14.0, 192.0, 236.0, 20.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 34.0, 22.0, 194.0, 232.0, 16.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 44.0, 26.0, 194.0, 228.0, 15.0, 38.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 1.0, 6.0, 9.0, 0.24, (16, 22, 48))

    paper = lin_grad(x, y, (px0, py0), (px1, py1), (254, 253, 250), (242, 240, 233))
    canvas = over(canvas, paper, m_page)

    # Page edge hairline (strong cool-slate at small sizes so it stays crisp on light taskbars)
    edge_w = 9.0 if small else (5.2 if medium else 2.4)
    m_edge = aa_mask(np.maximum(d_page, -(d_page + edge_w)), px)
    edge_col = (94, 108, 142) if (small or medium) else (138, 150, 174)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.92 if medium else 0.78)))

    # Folded corner
    flap_verts = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(
        sdf_polygon(x, y, flap_verts),
        sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 5.0),
    )
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.18, (20, 28, 56), clip_mask=m_page)
    flap_col = lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (226, 224, 216), (204, 202, 194))
    canvas = over(canvas, flap_col, m_flap)
    m_flap_edge = aa_mask(np.maximum(d_flap, -(d_flap + (edge_w * 0.85))), px) * m_page
    canvas = over(canvas, edge_col, m_flap_edge * 0.72)

    # Quiet text bars on upper page (skipped at <=20px)
    if not small:
        d_l1 = sdf_rrect(x, y, px0 + 22.0, py0 + 38.0, px1 - fold - 12.0, py0 + 47.0, 4.5)
        d_l2 = sdf_rrect(x, y, px0 + 22.0, py0 + 58.0, px1 - 36.0, py0 + 67.0, 4.5)
        canvas = over(canvas, (190, 197, 210), aa_mask(d_l1, px) * m_page)
        canvas = over(canvas, (204, 210, 222), aa_mask(d_l2, px) * m_page)

    # 3. Graceful calligraphic ink stroke sweeping smoothly into the nib tip
    # Fine entry on the left, confident broad swell through the curve, crisp arrival at the nib tip
    tip_x, tip_y = (126.0, 174.0) if small else (136.0, 168.0)
    b_pts = []
    p0, p1, p2, p3 = (64.0, 196.0), (88.0, 196.0), (108.0, 182.0), (tip_x, tip_y)
    steps = 44
    for i in range(steps + 1):
        t = i / steps
        mt = 1.0 - t
        bx = mt**3 * p0[0] + 3 * mt**2 * t * p1[0] + 3 * mt * t**2 * p2[0] + t**3 * p3[0]
        by = mt**3 * p0[1] + 3 * mt**2 * t * p1[1] + 3 * mt * t**2 * p2[1] + t**3 * p3[1]
        # True calligraphic taper: sharp fine start (0.14) -> confident mid swell (1.0) -> fine tip (0.28)
        taper = (t ** 0.75) * ((1.0 - 0.78 * (t ** 2.2)))
        rad_i = (11.5 if small else 8.2) * max(0.14, taper * 1.45)
        b_pts.append((bx, by, rad_i))

    d_ink = np.full_like(x, 1e6, dtype=np.float32)
    for i in range(len(b_pts) - 1):
        ax, ay, ar = b_pts[i]
        bx_, by_, br = b_pts[i + 1]
        seg_d = sdf_segment(x, y, ax, ay, bx_, by_) - 0.5 * (ar + br)
        d_ink = np.minimum(d_ink, seg_d)
    m_ink = aa_mask(d_ink, px) * m_page
    ink_col = lin_grad(x, y, (64, 198), (tip_x, tip_y), (68, 84, 224), (92, 62, 212))
    canvas = over(canvas, ink_col, m_ink)

    # 4. Sculpted Fountain Pen Nib (smooth curved shoulders, breaking past the right edge)
    ang = math.radians(-42.0)  # +u goes up-right along nib axis, +v perpendicular
    ca, sa = math.cos(ang), math.sin(ang)
    dx, dy = x - tip_x, y - tip_y
    u = dx * ca + dy * sa
    v = -dx * sa + dy * ca

    sc = 1.14 if small else 1.0

    def sample_cubic(c0, c1, c2, c3, n_seg):
        pts = []
        for k in range(n_seg + 1):
            t = k / n_seg
            mt = 1.0 - t
            pts.append((
                (mt**3 * c0[0] + 3 * mt**2 * t * c1[0] + 3 * mt * t**2 * c2[0] + t**3 * c3[0]) * sc,
                (mt**3 * c0[1] + 3 * mt**2 * t * c1[1] + 3 * mt * t**2 * c2[1] + t**3 * c3[1]) * sc,
            ))
        return pts

    # Smooth curved nib flank: tip (0,0) -> graceful convex sweep to shoulder (52, 32) -> concave waist to base (98, 19)
    upper_a = sample_cubic((0.0, 0.0), (14.0, 5.0), (34.0, 26.0), (54.0, 32.0), 10)
    upper_b = sample_cubic((54.0, 32.0), (68.0, 34.0), (84.0, 24.0), (98.0, 19.0), 8)
    pos_flank = upper_a + upper_b[1:]
    neg_flank = [(pu, -pv) for (pu, pv) in reversed(pos_flank[1:])]
    nib_poly = pos_flank + neg_flank

    d_nib = sdf_polygon(u, v, nib_poly)
    # Rounded brass collar / ferrule behind the nib (u: 97..121)
    collar_poly = [
        (96.0 * sc, 21.0 * sc),
        (119.0 * sc, 22.5 * sc),
        (119.0 * sc, -22.5 * sc),
        (96.0 * sc, -21.0 * sc),
    ]
    d_collar = sdf_polygon(u, v, collar_poly) - 1.5
    d_pen_all = np.minimum(d_nib, d_collar)
    m_pen_all = aa_mask(d_pen_all, px)

    # Soft cast shadow of nib onto the page
    if not small:
        canvas = add_shadow(canvas, m_pen_all, px, 2.0, 8.0, 7.0, 0.28, (14, 16, 46))

    # Two faceted halves of the nib (upper-left facet catches studio light, lower-right facet deep indigo)
    m_nib = aa_mask(d_nib, px)
    left_facet = lin_grad(u, v, (0, -30), (100, 10), (102, 92, 240), (54, 44, 174))
    right_facet = lin_grad(u, v, (0, 0), (100, 32), (46, 36, 148), (22, 16, 84))
    ridge_blend = aa_mask(v, px)[..., None]
    nib_rgb = left_facet * ridge_blend + right_facet * (1.0 - ridge_blend)
    canvas = over(canvas, nib_rgb, m_nib)

    # Specular crisp highlight along the upper edge of the nib
    if not small:
        m_nib_rim = aa_mask(np.maximum(d_nib, -(d_nib + 2.0)), px) * (v < 0).astype(np.float32)
        canvas = over(canvas, (202, 210, 255), m_nib_rim * 0.44)

    # Metallic warm brass-gold collar with soft cylinder shading
    m_collar = aa_mask(d_collar, px)
    collar_rgb = lin_grad(u, v, (106, -24), (106, 24), (254, 220, 114), (188, 124, 22))
    canvas = over(canvas, collar_rgb, m_collar)

    # Gold central slit (starts cleanly inside the dark pointed tip!) + circular breather hole
    hole_u = 49.0 * sc
    hole_r = 9.0 if small else (7.4 if medium else 6.4)
    d_hole = np.hypot(u - hole_u, v) - hole_r
    slit_w = 2.8 if small else (2.1 if medium else 1.55)
    d_slit = sdf_segment(u, v, 8.5 * sc, 0.0, hole_u, 0.0) - slit_w
    m_gold_detail = aa_mask(np.minimum(d_hole, d_slit), px) * m_nib
    gold_rgb = lin_grad(u, v, (12, -8), (60, 8), (255, 234, 148), (238, 174, 44))
    canvas = over(canvas, gold_rgb, m_gold_detail)

    return downsample(canvas, size)


# ===========================================================================
# OPTION 3: "RIBBON E"
# Architectural obsidian squircle + crisp rim + geometric folded-page E + coral silk bar
# ===========================================================================

def draw_ribbon_e(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Obsidian-slate squircle tile
    r_tile = 118.0 if small else (114.0 if medium else 110.0)
    d_tile = sdf_superellipse(x, y, 128.0, 128.0, r_tile, r_tile, n=5.0)
    m_tile = aa_mask(d_tile, px)

    if not small and not medium:
        canvas = add_shadow(canvas, m_tile, px, 0.0, 5.0, 8.0, 0.30, (6, 8, 16))

    bg = lin_grad(x, y, (30, 18), (228, 242), (38, 46, 66), (15, 19, 30))
    # Subtle soft top radial sheen
    sheen = np.exp(-(((x - 104.0) / 130.0) ** 2 + ((y - 46.0) / 88.0) ** 2))[..., None]
    bg = np.clip(bg + sheen * np.array([0.055, 0.068, 0.095], dtype=np.float32), 0.0, 1.0)
    canvas = over(canvas, bg, m_tile)

    # Crisp machined silver-slate bevel rim (ensures unmistakable separation on dark taskbars!)
    rim_w = 7.5 if small else (4.8 if medium else 3.0)
    m_rim = aa_mask(np.maximum(d_tile, -(d_tile + rim_w)), px)
    rim_col = lin_grad(x, y, (128, 14), (128, 242), (124, 138, 168), (58, 66, 86))
    canvas = over(canvas, rim_col, m_rim * (0.95 if small else 0.85))

    # 2. Geometric Folded-Paper "E" Monogram
    # Spine + top arm + bottom arm in luminous warm white; top-right corner cut cleanly at 45 deg (fold == ah!)
    if small:
        ex0, ey0, ex1, ey1 = 56.0, 50.0, 200.0, 206.0
        sw, ah, r_arm = 34.0, 34.0, 6.5
        my0, my1, mx1 = 111.0, 145.0, 188.0
    elif medium:
        ex0, ey0, ex1, ey1 = 62.0, 54.0, 196.0, 202.0
        sw, ah, r_arm = 31.0, 31.0, 6.0
        my0, my1, mx1 = 113.0, 143.0, 184.0
    else:
        ex0, ey0, ex1, ey1 = 66.0, 56.0, 194.0, 200.0
        sw, ah, r_arm = 29.0, 29.0, 6.0
        my0, my1, mx1 = 114.0, 142.0, 182.0

    fold = ah  # exact 1:1 match so the 45-deg fold meets the bottom-right corner with zero nub!

    # White structural C-frame (left spine + top arm with clean 45-deg page-corner cut + bottom arm)
    d_spine = sdf_rrect(x, y, ex0, ey0, ex0 + sw, ey1, r_arm)
    d_top_raw = sdf_rrect(x, y, ex0, ey0, ex1, ey0 + ah, r_arm)
    d_top_cut = sdf_halfplane(x, y, ex1 - fold, ey0, 1.0, -1.0)
    d_top = np.maximum(d_top_raw, d_top_cut)
    d_bot = sdf_rrect(x, y, ex0, ey1 - ah, ex1, ey1, r_arm)

    d_white_e = np.minimum(d_spine, np.minimum(d_top, d_bot))
    m_white_e = aa_mask(d_white_e, px)

    if not small:
        canvas = add_shadow(canvas, m_white_e, px, 0.0, 6.0, 9.0, 0.45, (4, 6, 14), clip_mask=m_tile)

    white_rgb = lin_grad(x, y, (ex0, ey0), (ex1, ey1), (255, 255, 255), (228, 234, 246))
    canvas = over(canvas, white_rgb, m_white_e)

    # Clean folded-corner triangle at top-right of the top arm + subtle crease shadow
    if not small:
        flap_v = [(ex1 - fold, ey0), (ex1, ey0 + fold), (ex1 - fold, ey0 + fold)]
        d_eflap = sdf_polygon(x, y, flap_v)
        m_eflap = aa_mask(d_eflap, px) * m_white_e
        canvas = add_shadow(canvas, m_eflap, px, -3.0, 2.0, 3.0, 0.16, (20, 30, 56), clip_mask=m_white_e)
        eflap_col = lin_grad(x, y, (ex1 - fold, ey0 + fold), (ex1, ey0), (200, 210, 230), (176, 188, 214))
        canvas = over(canvas, eflap_col, m_eflap)

    # 3. Hero middle arm: Vivid Warm Silk-Coral Bookmark Ribbon wrapping cleanly from the left spine edge!
    # Flush with the spine's left edge (ex0) so it wraps intentionally like a real silk bookmark ribbon,
    # ending in a matching 45-deg diagonal chisel cut on the bottom-right.
    d_mid_raw = sdf_rrect(x, y, ex0, my0, mx1, my1, r_arm * 0.9)
    chisel = 15.0 if not small else 12.0
    d_mid_cut = sdf_halfplane(x, y, mx1 - chisel, my1, 1.0, 1.0)
    d_mid = np.maximum(d_mid_raw, d_mid_cut)
    m_mid = aa_mask(d_mid, px)

    # Subtle ambient drop shadow cast onto both the dark tile and the white spine underneath
    if not small:
        canvas = add_shadow(canvas, m_mid, px, 0.0, 5.0, 8.5, 0.36, (18, 12, 16), clip_mask=m_tile)

    # Rich luminous coral-vermilion to warm amber-coral gradient
    coral_rgb = lin_grad(x, y, (ex0, my0), (mx1, my1), (255, 134, 68), (235, 68, 52))
    canvas = over(canvas, coral_rgb, m_mid)

    # Subtle fold crease right where the ribbon turns past the spine edge (adds tactile book-ribbon craft)
    if not small:
        spine_band = aa_mask(sdf_rrect(x, y, ex0, my0, ex0 + sw, my1, 0.0), px) * m_mid
        canvas = over(canvas, (255, 224, 198), spine_band * 0.12)
        m_mid_top = aa_mask(np.maximum(d_mid, -(d_mid + 2.2)), px) * (y < (my0 + my1) * 0.5).astype(np.float32)
        canvas = over(canvas, (255, 224, 198), m_mid_top * 0.42)

    return downsample(canvas, size)


# ===========================================================================
# Specimen / Comparison Sheet
# ===========================================================================

def load_font(name: str, px: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    path = WIN_FONTS / name
    try:
        return ImageFont.truetype(str(path), px)
    except OSError:
        return ImageFont.load_default(px)


def build_comparison_sheet(
    variants: list[tuple[str, str, str, dict[int, Image.Image]]],
    out_path: Path,
) -> None:
    """Renders a clean gallery plate comparing all 3 alternatives at large, true, taskbar and magnified sizes."""
    W, H = 2640, 1840
    bg = (246, 245, 241, 255)
    ink = (26, 31, 42, 255)
    muted = (96, 104, 118, 255)
    rule = (218, 215, 207, 255)

    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)

    f_title = load_font("segoeuib.ttf", 44)
    f_sub = load_font("segoeui.ttf", 24)
    f_h2 = load_font("segoeuib.ttf", 34)
    f_body = load_font("segoeui.ttf", 21)
    f_mono = load_font("consola.ttf", 18)

    # Header
    d.text((84, 56), "EcclesiaPDF — Alternative App Icon Concepts", font=f_title, fill=ink)
    d.text(
        (84, 114),
        "Three distinct directions rendered per pixel grid (16 px to 1024 px), shown on light, dark taskbar and magnified lattices",
        font=f_sub,
        fill=muted,
    )

    # Small reference badge of current Marginalia on top-right for easy comparison
    cur_path = ICON_DIR / "png" / "ecclesiapdf-128.png"
    if cur_path.exists():
        cur_im = Image.open(cur_path).convert("RGBA").resize((76, 76), Image.LANCZOS)
        rx = W - 360
        d.rounded_rectangle([rx - 18, 48, W - 84, 144], radius=12, fill=(238, 236, 230, 255), outline=rule, width=1)
        img.alpha_composite(cur_im, (rx, 58))
        d.text((rx + 92, 68), "Current (DD28)", font=f_mono, fill=muted)
        d.text((rx + 92, 94), "Marginalia", font=f_body, fill=ink)

    d.line([(84, 168), (W - 84, 168)], fill=rule, width=2)

    col_w = (W - 168 - 80) // 3  # 3 columns with 40px gaps
    for idx, (label, subtitle, rationale, icons) in enumerate(variants):
        cx0 = 84 + idx * (col_w + 40)
        cx_mid = cx0 + col_w // 2

        if idx > 0:
            d.line([(cx0 - 20, 196), (cx0 - 20, H - 72)], fill=rule, width=1)

        # Column title & subtitle
        d.text((cx0, 196), label, font=f_h2, fill=ink)
        d.text((cx0, 240), subtitle, font=f_body, fill=muted)

        # Hero 384px render on subtle elevated light studio surface
        hero_box = [cx0, 286, cx0 + col_w, 720]
        d.rounded_rectangle(hero_box, radius=18, fill=(252, 252, 250, 255), outline=rule, width=1)
        big = icons[512].resize((360, 360), Image.LANCZOS)
        img.alpha_composite(big, (cx_mid - 180, 322))

        # Short rationale (2 lines)
        d.text((cx0, 742), rationale, font=f_body, fill=ink)

        # Row 1: True-size lineup on light surface (128, 64, 48, 32, 24, 16)
        d.text((cx0, 818), "TRUE SIZE ON LIGHT", font=f_mono, fill=muted)
        row_sizes = [128, 64, 48, 32, 24, 16]
        tx = cx0 + 16
        base_y = 984
        for sz in row_sizes:
            img.alpha_composite(icons[sz], (tx, base_y - sz))
            d.text((tx, base_y + 10), f"{sz}", font=f_mono, fill=muted)
            tx += sz + 36

        # Row 2: Windows 11 Dark Taskbar strip (#202022)
        bar_y0, bar_y1 = 1044, 1152
        d.rounded_rectangle([cx0, bar_y0, cx0 + col_w, bar_y1], radius=12, fill=(32, 32, 34, 255))
        tx = cx0 + 28
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, (bar_y0 + bar_y1 - sz) // 2))
            tx += sz + 44
        d.text((cx0, bar_y1 + 10), "DARK TASKBAR (#202022)", font=f_mono, fill=muted)

        # Row 3: Windows 11 Light Taskbar strip (#F3F3F3)
        lbar_y0, lbar_y1 = 1196, 1296
        d.rounded_rectangle(
            [cx0, lbar_y0, cx0 + col_w, lbar_y1],
            radius=12,
            fill=(243, 243, 243, 255),
            outline=(222, 222, 222, 255),
            width=1,
        )
        tx = cx0 + 28
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, (lbar_y0 + lbar_y1 - sz) // 2))
            tx += sz + 44
        d.text((cx0, lbar_y1 + 10), "LIGHT TASKBAR (#F3F3F3)", font=f_mono, fill=muted)

        # Row 4: Magnified pixel grids (16 px x16 and 32 px x8 -> both 256px square)
        d.text((cx0, 1360), "PIXEL GRID INSPECTION (16 PX  /  32 PX)", font=f_mono, fill=muted)
        gy = 1394
        for gi, (sz, mag) in enumerate([(16, 16), (32, 8)]):
            gx = cx0 + gi * (256 + 44)
            plate = icons[sz].resize((sz * mag, sz * mag), Image.NEAREST)
            # Checkerboard subtle backdrop
            d.rectangle([gx, gy, gx + 256, gy + 256], fill=(255, 255, 255, 255), outline=rule, width=1)
            img.alpha_composite(plate, (gx, gy))
            for k in range(sz + 1):
                d.line([(gx + k * mag, gy), (gx + k * mag, gy + 256)], fill=(230, 228, 222, 255), width=1)
                d.line([(gx, gy + k * mag), (gx + 256, gy + k * mag)], fill=(230, 228, 222, 255), width=1)
            d.text((gx, gy + 266), f"{sz}x{sz}  ({mag}x)", font=f_mono, fill=muted)

    img.convert("RGB").save(out_path, optimize=True)


def main() -> None:
    specs: list[tuple[str, str, str, str, Callable[[int], Image.Image]]] = [
        (
            "folio",
            "1. Folio",
            "Cobalt squircle, stacked pages & amber highlight",
            "Read-mode cobalt tile with crisp layered paper and\nan amber-gold highlight band. Pops on every taskbar.",
            draw_folio,
        ),
        (
            "nib_and_leaf",
            "2. Nib & Leaf",
            "Unframed Fluent page, fountain nib & fluid ink",
            "Open silhouette pairing layered ivory pages with a\nsculpted indigo-violet nib, gold slit and ink curve.",
            draw_nib_and_leaf,
        ),
        (
            "ribbon_e",
            "3. Ribbon E",
            "Obsidian squircle, folded-page E & coral silk bar",
            "Minimal architectural monogram with dog-ear corner\nfold and silk-coral bookmark bar. Zero visual noise.",
            draw_ribbon_e,
        ),
    ]

    rendered = []
    for slug, label, sub, rationale, fn in specs:
        out_dir = HERE / slug
        out_dir.mkdir(parents=True, exist_ok=True)
        icons = {sz: fn(sz) for sz in SIZES}
        for sz in SIZES:
            icons[sz].save(out_dir / f"ecclesiapdf-{slug}-{sz}.png", optimize=True)
        rendered.append((label, sub, rationale, icons))
        print(f"rendered {slug} ({len(SIZES)} sizes)")

    sheet_path = HERE / "comparison-sheet.png"
    build_comparison_sheet(rendered, sheet_path)
    print("wrote", sheet_path)


if __name__ == "__main__":
    main()
