"""Generates 3 refined variants of the 'Nib & Leaf' icon concept for EcclesiaPDF.

    python design/icon/alternatives/render_nib_variants.py

Does NOT modify the active icon (design/icon/render_icon.py or packaging/windows/ecclesiapdf.ico).
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Callable

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

# Reuse exact math/SDF/compositing helpers from render_alternatives.py
from render_alternatives import (
    SIZES,
    WIN_FONTS,
    aa_mask,
    add_shadow,
    downsample,
    lin_grad,
    load_font,
    make_grid,
    over,
    render_ss,
    sdf_halfplane,
    sdf_polygon,
    sdf_rot_rrect,
    sdf_rrect,
    sdf_segment,
    sdf_superellipse,
)

HERE = Path(__file__).resolve().parent
OUT_ROOT = HERE / "nib_variants"


def sample_cubic(
    c0: tuple[float, float],
    c1: tuple[float, float],
    c2: tuple[float, float],
    c3: tuple[float, float],
    n_seg: int,
    scale: float = 1.0,
) -> list[tuple[float, float]]:
    pts = []
    for k in range(n_seg + 1):
        t = k / n_seg
        mt = 1.0 - t
        pts.append(
            (
                (mt**3 * c0[0] + 3 * mt**2 * t * c1[0] + 3 * mt * t**2 * c2[0] + t**3 * c3[0]) * scale,
                (mt**3 * c0[1] + 3 * mt**2 * t * c1[1] + 3 * mt * t**2 * c2[1] + t**3 * c3[1]) * scale,
            )
        )
    return pts


def draw_sculpted_nib(
    canvas: np.ndarray,
    x: np.ndarray,
    y: np.ndarray,
    px: int,
    tip_x: float,
    tip_y: float,
    angle_deg: float,
    scale: float,
    small: bool,
    medium: bool,
    left_c0: tuple[float, float, float],
    left_c1: tuple[float, float, float],
    right_c0: tuple[float, float, float],
    right_c1: tuple[float, float, float],
    rim_col: tuple[float, float, float],
    rim_alpha: float,
    collar_c0: tuple[float, float, float],
    collar_c1: tuple[float, float, float],
    band_col: tuple[float, float, float],
    slit_c0: tuple[float, float, float],
    slit_c1: tuple[float, float, float],
    tip_glint: tuple[float, float, float] | None = None,
    shadow_clip: np.ndarray | None = None,
    elongated: bool = False,
) -> np.ndarray:
    """Renders a precision faceted fountain-pen nib with smooth curved shoulders,
    engraved dual-ring ferrule/collar, breather hole, central ink slit, and cast shadow."""
    ang = math.radians(angle_deg)
    ca, sa = math.cos(ang), math.sin(ang)
    dx, dy = x - tip_x, y - tip_y
    u = dx * ca + dy * sa
    v = -dx * sa + dy * ca

    if elongated:
        # Sleeker architect-style fountain nib
        upper_a = sample_cubic((0.0, 0.0), (16.0, 4.0), (36.0, 24.0), (56.0, 29.0), 12, scale)
        upper_b = sample_cubic((56.0, 29.0), (70.0, 31.0), (86.0, 22.0), (102.0, 17.5), 10, scale)
        base_u, collar_end, collar_hw = 100.0 * scale, 123.0 * scale, 19.5 * scale
        hole_u = 52.0 * scale
    else:
        # Classic balanced broad fountain nib
        upper_a = sample_cubic((0.0, 0.0), (14.0, 4.5), (34.0, 26.0), (54.0, 32.0), 12, scale)
        upper_b = sample_cubic((54.0, 32.0), (68.0, 34.0), (84.0, 24.0), (98.0, 19.0), 10, scale)
        base_u, collar_end, collar_hw = 96.0 * scale, 120.0 * scale, 21.5 * scale
        hole_u = 49.0 * scale

    pos_flank = upper_a + upper_b[1:]
    neg_flank = [(pu, -pv) for (pu, pv) in reversed(pos_flank[1:])]
    nib_poly = pos_flank + neg_flank

    d_nib = sdf_polygon(u, v, nib_poly)
    collar_poly = [
        (base_u, collar_hw),
        (collar_end, collar_hw + 1.5 * scale),
        (collar_end, -(collar_hw + 1.5 * scale)),
        (base_u, -collar_hw),
    ]
    d_collar = sdf_polygon(u, v, collar_poly) - 1.5 * scale
    d_pen_all = np.minimum(d_nib, d_collar)
    m_pen_all = aa_mask(d_pen_all, px)

    if not small:
        canvas = add_shadow(canvas, m_pen_all, px, 2.0, 7.5, 7.0, 0.30, (12, 16, 42), clip_mask=shadow_clip)

    # Faceted nib body
    m_nib = aa_mask(d_nib, px)
    left_facet = lin_grad(u, v, (0, -30 * scale), (100 * scale, 10 * scale), left_c0, left_c1)
    right_facet = lin_grad(u, v, (0, 0), (100 * scale, 32 * scale), right_c0, right_c1)
    ridge_blend = aa_mask(v, px)[..., None]
    nib_rgb = left_facet * ridge_blend + right_facet * (1.0 - ridge_blend)
    canvas = over(canvas, nib_rgb, m_nib)

    # Subtle specular edge highlight on lit flank
    if not small:
        m_nib_rim = aa_mask(np.maximum(d_nib, -(d_nib + 2.0 * scale)), px) * (v < 0).astype(np.float32)
        canvas = over(canvas, rim_col, m_nib_rim * rim_alpha)

    # Metallic ferrule / collar with cylindrical shading + fine engraved double-ring detail
    m_collar = aa_mask(d_collar, px)
    collar_rgb = lin_grad(u, v, (base_u + 10 * scale, -24 * scale), (base_u + 10 * scale, 24 * scale), collar_c0, collar_c1)
    canvas = over(canvas, collar_rgb, m_collar)
    if not small and not medium:
        # Fine engraved ring crease near nib-collar seam
        ring_u = base_u + 5.5 * scale
        d_ring = np.abs(u - ring_u) - 1.1 * scale
        m_ring = aa_mask(d_ring, px) * m_collar
        canvas = over(canvas, band_col, m_ring * 0.42)
        # Subtle specular catchlight along upper cylinder of collar
        d_spec = np.abs(v + collar_hw * 0.48) - 2.6 * scale
        m_spec = aa_mask(d_spec, px) * m_collar
        canvas = over(canvas, (255, 252, 236), m_spec * 0.30)

    # Breather hole + precision ink slit (cleanly inside the pointed tip)
    hole_r = (8.8 if small else (7.2 if medium else 6.2)) * scale
    d_hole = np.hypot(u - hole_u, v) - hole_r
    slit_w = (2.7 if small else (2.0 if medium else 1.45)) * scale
    d_slit = sdf_segment(u, v, 8.5 * scale, 0.0, hole_u, 0.0) - slit_w
    m_slit = aa_mask(np.minimum(d_hole, d_slit), px) * m_nib
    slit_rgb = lin_grad(u, v, (10 * scale, -8 * scale), (58 * scale, 8 * scale), slit_c0, slit_c1)
    canvas = over(canvas, slit_rgb, m_slit)

    # Tiny polished iridium tip glint at large sizes
    if tip_glint is not None and not small and not medium:
        d_tip = np.hypot(u - 3.6 * scale, v) - 2.2 * scale
        canvas = over(canvas, tip_glint, aa_mask(d_tip, px) * m_nib * 0.85)

    return canvas


def draw_calligraphic_stroke(
    canvas: np.ndarray,
    x: np.ndarray,
    y: np.ndarray,
    px: int,
    p0: tuple[float, float],
    p1: tuple[float, float],
    p2: tuple[float, float],
    p3: tuple[float, float],
    max_r: float,
    c0: tuple[float, float, float],
    c1: tuple[float, float, float],
    clip_mask: np.ndarray,
) -> np.ndarray:
    """Smooth tapered calligraphic ink curve arriving cleanly at p3 (the nib tip)."""
    pts = []
    steps = 48
    for i in range(steps + 1):
        t = i / steps
        mt = 1.0 - t
        bx = mt**3 * p0[0] + 3 * mt**2 * t * p1[0] + 3 * mt * t**2 * p2[0] + t**3 * p3[0]
        by = mt**3 * p0[1] + 3 * mt**2 * t * p1[1] + 3 * mt * t**2 * p2[1] + t**3 * p3[1]
        # Smooth calligraphic pressure envelope: fine hairline start -> rich belly -> crisp arrival
        env = math.sin(math.pi * (0.04 + 0.90 * t)) ** 1.15
        rad = max_r * max(0.14, env)
        pts.append((bx, by, rad))

    d_ink = np.full_like(x, 1e6, dtype=np.float32)
    for i in range(len(pts) - 1):
        ax, ay, ar = pts[i]
        bx_, by_, br = pts[i + 1]
        d_ink = np.minimum(d_ink, sdf_segment(x, y, ax, ay, bx_, by_) - 0.5 * (ar + br))
    m_ink = aa_mask(d_ink, px) * clip_mask
    ink_rgb = lin_grad(x, y, p0, p3, c0, c1)
    return over(canvas, ink_rgb, m_ink)


# ===========================================================================
# VARIANT 1: "CLASSIC INDIGO" (Refined Open Fluent Silhouette)
# ===========================================================================

def draw_variant_classic(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Cool-periwinkle back sheet
    if not small:
        d_back = sdf_rot_rrect(x, y, 105.0, 122.0, 144.0, 192.0, 16.0, -5.5)
        m_back = aa_mask(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -1.0, 5.0, 8.0, 0.16, (18, 24, 52))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (220, 232, 254), (186, 204, 244))
        canvas = over(canvas, back_col, m_back)
        m_be = aa_mask(np.maximum(d_back, -(d_back + 2.2)), px)
        canvas = over(canvas, (124, 146, 204), m_be * 0.62)

    # 2. Hero ivory folded page
    if small:
        px0, py0, px1, py1, pr, fold = 22.0, 14.0, 190.0, 236.0, 20.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 34.0, 22.0, 194.0, 232.0, 16.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 44.0, 26.0, 194.0, 228.0, 15.0, 38.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 1.0, 6.0, 9.0, 0.24, (16, 22, 48))

    paper = lin_grad(x, y, (px0, py0), (px1, py1), (255, 254, 251), (242, 240, 233))
    canvas = over(canvas, paper, m_page)

    edge_w = 9.0 if small else (5.0 if medium else 2.4)
    edge_col = (84, 98, 138) if (small or medium) else (132, 146, 174)
    m_edge = aa_mask(np.maximum(d_page, -(d_page + edge_w)), px)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.94 if medium else 0.78)))

    # Corner fold
    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 5.0))
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.18, (20, 28, 56), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (224, 224, 218), (200, 202, 198)), m_flap)
    canvas = over(canvas, edge_col, aa_mask(np.maximum(d_flap, -(d_flap + edge_w * 0.8)), px) * m_page * 0.70)

    # Quiet text lines (refined 3-line rhythm, clear of the ink curve)
    if not small:
        d_l1 = sdf_rrect(x, y, px0 + 22.0, py0 + 36.0, px1 - fold - 12.0, py0 + 44.5, 4.2)
        d_l2 = sdf_rrect(x, y, px0 + 22.0, py0 + 55.0, px1 - 32.0, py0 + 63.5, 4.2)
        d_l3 = sdf_rrect(x, y, px0 + 22.0, py0 + 74.0, px1 - 68.0, py0 + 82.5, 4.2)
        canvas = over(canvas, (186, 194, 210), aa_mask(d_l1, px) * m_page)
        canvas = over(canvas, (198, 204, 218), aa_mask(d_l2, px) * m_page)
        canvas = over(canvas, (208, 213, 224), aa_mask(d_l3, px) * m_page)

    # 3. Graceful calligraphic S-wave ink stroke
    tip_x, tip_y = (126.0, 174.0) if small else (136.0, 166.0)
    canvas = draw_calligraphic_stroke(
        canvas,
        x,
        y,
        px,
        p0=(58.0, 196.0) if not small else (46.0, 202.0),
        p1=(84.0, 206.0),
        p2=(106.0, 174.0),
        p3=(tip_x, tip_y),
        max_r=11.0 if small else 7.8,
        c0=(56, 78, 224),
        c1=(94, 58, 216),
        clip_mask=m_page,
    )

    # 4. Sculpted Royal-Indigo Nib + Dual-Ring Satin Brass Ferrule
    canvas = draw_sculpted_nib(
        canvas,
        x,
        y,
        px,
        tip_x=tip_x,
        tip_y=tip_y,
        angle_deg=-42.0,
        scale=1.14 if small else 1.0,
        small=small,
        medium=medium,
        left_c0=(104, 94, 242),
        left_c1=(54, 44, 176),
        right_c0=(44, 34, 148),
        right_c1=(20, 15, 82),
        rim_col=(206, 214, 255),
        rim_alpha=0.45,
        collar_c0=(254, 222, 118),
        collar_c1=(184, 120, 20),
        band_col=(112, 66, 8),
        slit_c0=(255, 236, 154),
        slit_c1=(238, 172, 42),
        tip_glint=(255, 244, 200),
    )

    return downsample(canvas, size)


# ===========================================================================
# VARIANT 2: "GILDED TILE" (Framed Midnight-Cobalt Squircle + 18k Gold Nib)
# ===========================================================================

def draw_variant_gilded_tile(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Deep midnight-sapphire squircle (unbeatable presence on both light & dark taskbars)
    r_tile = 118.0 if small else (114.0 if medium else 110.0)
    d_tile = sdf_superellipse(x, y, 128.0, 128.0, r_tile, r_tile, n=5.0)
    m_tile = aa_mask(d_tile, px)

    if not small and not medium:
        canvas = add_shadow(canvas, m_tile, px, 0.0, 5.0, 8.0, 0.30, (6, 12, 34))

    bg = lin_grad(x, y, (28, 18), (228, 242), (34, 68, 168), (13, 24, 62))
    glow = np.exp(-(((x - 84.0) / 120.0) ** 2 + ((y - 52.0) / 92.0) ** 2))[..., None]
    bg = np.clip(bg + glow * np.array([0.06, 0.09, 0.15], dtype=np.float32), 0.0, 1.0)
    canvas = over(canvas, bg, m_tile)

    # Crisp cool-silver specular inner rim (separates cleanly on dark #202022 taskbars)
    rim_w = 6.5 if small else (4.2 if medium else 2.6)
    m_rim = aa_mask(np.maximum(d_tile, -(d_tile + rim_w)), px)
    rim_col = lin_grad(x, y, (128, 14), (128, 242), (160, 192, 255), (54, 76, 134))
    canvas = over(canvas, rim_col, m_rim * (0.78 if small else 0.62))

    # 2. Subtle translucent back sheet peeking left
    if not small:
        d_back = sdf_rot_rrect(x, y, 102.0, 128.0, 106.0, 144.0, 13.0, -6.5)
        m_back = aa_mask(d_back, px) * m_tile
        canvas = over(canvas, lin_grad(x, y, (46, 54), (156, 198), (176, 204, 255), (96, 134, 224)), m_back * 0.32)

    # 3. Crisp luminous white folded sheet inside the squircle
    if small:
        px0, py0, px1, py1, pr, fold = 42.0, 34.0, 182.0, 220.0, 15.0, 38.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 48.0, 38.0, 180.0, 214.0, 13.0, 36.0
    else:
        px0, py0, px1, py1, pr, fold = 54.0, 42.0, 178.0, 212.0, 13.0, 34.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 0.0, 6.5, 9.5, 0.40, (6, 12, 38), clip_mask=m_tile)

    canvas = over(canvas, lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 252), (240, 241, 236)), m_page)

    # Corner fold
    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 4.5))
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.22, (12, 24, 64), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (218, 228, 246), (186, 200, 230)), m_flap)

    # Subtle text lines
    if not small:
        d_l1 = sdf_rrect(x, y, px0 + 18.0, py0 + 32.0, px1 - fold - 10.0, py0 + 40.0, 4.0)
        d_l2 = sdf_rrect(x, y, px0 + 18.0, py0 + 50.0, px1 - 28.0, py0 + 58.0, 4.0)
        canvas = over(canvas, (176, 188, 210), aa_mask(d_l1, px) * m_page)
        canvas = over(canvas, (192, 202, 220), aa_mask(d_l2, px) * m_page)

    # 4. Rich cobalt-blue ink curve on the bright paper
    tip_x, tip_y = (124.0, 168.0) if small else (128.0, 164.0)
    canvas = draw_calligraphic_stroke(
        canvas,
        x,
        y,
        px,
        p0=(66.0, 188.0) if not small else (56.0, 194.0),
        p1=(88.0, 198.0),
        p2=(104.0, 172.0),
        p3=(tip_x, tip_y),
        max_r=10.5 if small else 7.2,
        c0=(26, 76, 204),
        c1=(54, 118, 246),
        clip_mask=m_page,
    )

    # 5. Sculpted 18k Champagne-Gold Fountain Nib + Deep Midnight-Navy Collar
    # Kept cleanly inside the squircle bounds with jewel-like metallic contrast!
    canvas = draw_sculpted_nib(
        canvas,
        x,
        y,
        px,
        tip_x=tip_x,
        tip_y=tip_y,
        angle_deg=-42.0,
        scale=1.02 if small else 0.90,
        small=small,
        medium=medium,
        left_c0=(255, 226, 122),
        left_c1=(232, 168, 36),
        right_c0=(212, 146, 24),
        right_c1=(154, 94, 12),
        rim_col=(255, 250, 218),
        rim_alpha=0.65,
        collar_c0=(56, 86, 176),
        collar_c1=(16, 28, 72),
        band_col=(180, 208, 255),
        slit_c0=(26, 46, 118),
        slit_c1=(12, 22, 64),
        tip_glint=(255, 252, 236),
        shadow_clip=m_tile,
    )

    return downsample(canvas, size)


# ===========================================================================
# VARIANT 3: "ATELIER TEAL" (Open Silhouette: Gunmetal Nib, Emerald Ink, Sage Stack)
# ===========================================================================

def draw_variant_atelier_teal(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Soft sage-emerald back sheet
    if not small:
        d_back = sdf_rot_rrect(x, y, 105.0, 122.0, 144.0, 192.0, 16.0, -5.8)
        m_back = aa_mask(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -1.0, 5.0, 8.0, 0.15, (14, 34, 30))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (212, 240, 230), (170, 218, 204))
        canvas = over(canvas, back_col, m_back)
        m_be = aa_mask(np.maximum(d_back, -(d_back + 2.2)), px)
        canvas = over(canvas, (100, 162, 144), m_be * 0.65)

    # 2. Hero crisp paper sheet
    if small:
        px0, py0, px1, py1, pr, fold = 22.0, 14.0, 190.0, 236.0, 20.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 34.0, 22.0, 194.0, 232.0, 16.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 44.0, 26.0, 194.0, 228.0, 15.0, 38.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 1.0, 6.0, 9.0, 0.24, (14, 24, 28))

    canvas = over(canvas, lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 253), (242, 243, 239)), m_page)

    edge_w = 9.0 if small else (5.0 if medium else 2.4)
    edge_col = (72, 90, 96) if (small or medium) else (128, 142, 146)
    m_edge = aa_mask(np.maximum(d_page, -(d_page + edge_w)), px)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.94 if medium else 0.78)))

    # Corner fold
    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 5.0))
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.18, (16, 30, 32), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (222, 230, 226), (196, 208, 204)), m_flap)
    canvas = over(canvas, edge_col, aa_mask(np.maximum(d_flap, -(d_flap + edge_w * 0.8)), px) * m_page * 0.70)

    # Quiet top line + delicate warm amber-apricot highlight bar (subtle Annotate nod!)
    if not small:
        d_l1 = sdf_rrect(x, y, px0 + 22.0, py0 + 38.0, px1 - fold - 12.0, py0 + 46.5, 4.2)
        canvas = over(canvas, (182, 192, 194), aa_mask(d_l1, px) * m_page)
        # Warm soft highlight strip behind line 2
        d_hl = sdf_rrect(x, y, px0 + 18.0, py0 + 54.0, px1 - 38.0, py0 + 70.0, 5.5)
        canvas = over(canvas, (254, 216, 108), aa_mask(d_hl, px) * m_page * 0.72)
        d_l2 = sdf_rrect(x, y, px0 + 22.0, py0 + 58.0, px1 - 44.0, py0 + 66.0, 4.0)
        canvas = over(canvas, (52, 64, 72), aa_mask(d_l2, px) * m_page * 0.78)

    # 3. Rich emerald-teal calligraphic ink stroke
    tip_x, tip_y = (126.0, 174.0) if small else (136.0, 166.0)
    canvas = draw_calligraphic_stroke(
        canvas,
        x,
        y,
        px,
        p0=(58.0, 196.0) if not small else (46.0, 202.0),
        p1=(86.0, 206.0),
        p2=(106.0, 174.0),
        p3=(tip_x, tip_y),
        max_r=11.0 if small else 7.8,
        c0=(16, 168, 120),
        c1=(10, 118, 106),
        clip_mask=m_page,
    )

    # 4. Precision Gunmetal-Obsidian Architect's Nib + Warm Copper-Gold Collar
    canvas = draw_sculpted_nib(
        canvas,
        x,
        y,
        px,
        tip_x=tip_x,
        tip_y=tip_y,
        angle_deg=-42.0,
        scale=1.12 if small else 1.0,
        small=small,
        medium=medium,
        left_c0=(72, 86, 108),
        left_c1=(36, 44, 58),
        right_c0=(30, 38, 50),
        right_c1=(14, 18, 26),
        rim_col=(188, 204, 224),
        rim_alpha=0.42,
        collar_c0=(246, 186, 102),
        collar_c1=(168, 98, 28),
        band_col=(84, 44, 10),
        slit_c0=(252, 214, 134),
        slit_c1=(218, 146, 48),
        tip_glint=(234, 244, 248),
        elongated=True,
    )

    return downsample(canvas, size)


# ===========================================================================
# Comparison Plate
# ===========================================================================

def build_plate(
    variants: list[tuple[str, str, str, dict[int, Image.Image]]],
    out_path: Path,
) -> None:
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

    d.text((84, 56), "EcclesiaPDF — Nib & Leaf Variants", font=f_title, fill=ink)
    d.text(
        (84, 114),
        "Three variations of the Nib & Leaf direction: open Fluent silhouettes vs. framed midnight-cobalt tile (16 px to 1024 px)",
        font=f_sub,
        fill=muted,
    )
    d.line([(84, 168), (W - 84, 168)], fill=rule, width=2)

    col_w = (W - 168 - 80) // 3
    for idx, (label, subtitle, rationale, icons) in enumerate(variants):
        cx0 = 84 + idx * (col_w + 40)
        cx_mid = cx0 + col_w // 2

        if idx > 0:
            d.line([(cx0 - 20, 196), (cx0 - 20, H - 72)], fill=rule, width=1)

        d.text((cx0, 196), label, font=f_h2, fill=ink)
        d.text((cx0, 240), subtitle, font=f_body, fill=muted)

        hero_box = [cx0, 286, cx0 + col_w, 720]
        d.rounded_rectangle(hero_box, radius=18, fill=(252, 252, 250, 255), outline=rule, width=1)
        big = icons[512].resize((360, 360), Image.LANCZOS)
        img.alpha_composite(big, (cx_mid - 180, 322))

        d.text((cx0, 742), rationale, font=f_body, fill=ink)

        # True-size lineup on light
        d.text((cx0, 818), "TRUE SIZE ON LIGHT", font=f_mono, fill=muted)
        tx = cx0 + 16
        base_y = 984
        for sz in [128, 64, 48, 32, 24, 16]:
            img.alpha_composite(icons[sz], (tx, base_y - sz))
            d.text((tx, base_y + 10), f"{sz}", font=f_mono, fill=muted)
            tx += sz + 36

        # Dark taskbar strip (#202022)
        bar_y0, bar_y1 = 1044, 1152
        d.rounded_rectangle([cx0, bar_y0, cx0 + col_w, bar_y1], radius=12, fill=(32, 32, 34, 255))
        tx = cx0 + 28
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, (bar_y0 + bar_y1 - sz) // 2))
            tx += sz + 44
        d.text((cx0, bar_y1 + 10), "DARK TASKBAR (#202022)", font=f_mono, fill=muted)

        # Light taskbar strip (#F3F3F3)
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

        # Magnified pixel grids (16 px & 32 px)
        d.text((cx0, 1360), "PIXEL GRID INSPECTION (16 PX  /  32 PX)", font=f_mono, fill=muted)
        gy = 1394
        for gi, (sz, mag) in enumerate([(16, 16), (32, 8)]):
            gx = cx0 + gi * (256 + 44)
            plate = icons[sz].resize((sz * mag, sz * mag), Image.NEAREST)
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
            "classic_indigo",
            "A. Classic Indigo",
            "Open Fluent silhouette, royal-indigo nib & brass ferrule",
            "Refined Nib & Leaf: dual-ring satin brass collar,\niridium tip glint, 3-line rhythm and fluid S-curve ink.",
            draw_variant_classic,
        ),
        (
            "gilded_tile",
            "B. Gilded Tile",
            "Midnight-cobalt squircle, 18k gold nib & cobalt ink",
            "Framed for maximum dock & taskbar punch: gleaming\ngold nib drawing deep cobalt ink over crisp white paper.",
            draw_variant_gilded_tile,
        ),
        (
            "atelier_teal",
            "C. Atelier Teal",
            "Open silhouette, obsidian nib, emerald ink & amber highlight",
            "Editorial instrument feel: sleek gunmetal-obsidian nib\nwith copper collar, sage back sheet and teal ink stroke.",
            draw_variant_atelier_teal,
        ),
    ]

    rendered = []
    for slug, label, sub, rationale, fn in specs:
        out_dir = OUT_ROOT / slug
        out_dir.mkdir(parents=True, exist_ok=True)
        icons = {sz: fn(sz) for sz in SIZES}
        for sz in SIZES:
            icons[sz].save(out_dir / f"ecclesiapdf-{slug}-{sz}.png", optimize=True)
        rendered.append((label, sub, rationale, icons))
        print(f"rendered {slug} ({len(SIZES)} sizes)")

    plate_path = OUT_ROOT / "nib-variants-sheet.png"
    build_plate(rendered, plate_path)
    print("wrote", plate_path)


if __name__ == "__main__":
    main()
