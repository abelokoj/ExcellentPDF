"""Generates 3 macOS TextEdit-inspired 'Nib & Leaf' icon variants with the inscribed text:
    "In the Beginning, God created..."

    python design/icon/alternatives/render_nib_inscribed.py

Does NOT modify the active icon (design/icon/render_icon.py or packaging/windows/ecclesiapdf.ico).
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Callable

import numpy as np
from PIL import Image, ImageDraw, ImageFont

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
    sdf_superellipse,
    to_pil,
)
from render_nib_variants import draw_calligraphic_stroke, draw_sculpted_nib

HERE = Path(__file__).resolve().parent
OUT_ROOT = HERE / "nib_inscribed"


def composite_inscribed_text(
    canvas: np.ndarray,
    x: np.ndarray,
    y: np.ndarray,
    px: int,
    px0: float,
    py0: float,
    px1: float,
    py1: float,
    fold: float,
    m_page: np.ndarray,
    size: int,
    font_file: str,
    ink_rgb: tuple[int, int, int],
    rule_rgb: tuple[int, int, int],
    highlight_first: bool = False,
) -> np.ndarray:
    """Renders delicate macOS TextEdit-style ruled paper lines and the crisp inscribed
    editorial text 'In the Beginning, God created...' at sizes >= 64 px, falling back
    cleanly to crisp typographic bars at small icon sizes (<= 48 px)."""
    scale = px / 256.0
    small = size <= 20

    if small:
        return canvas

    # 1. Subtle TextEdit-style horizontal paper rules across the page (sizes >= 40 px)
    if size >= 40:
        margin_l = px0 + 13.0
        margin_r = px1 - 13.0
        line_ys = [py0 + 44.0, py0 + 68.0, py0 + 92.0, py0 + 116.0, py0 + 140.0, py0 + 164.0]
        d_rules = np.full_like(x, 1e6, dtype=np.float32)
        rule_h = max(0.55, 0.85 * (256.0 / px))
        for ly in line_ys:
            if ly > py1 - 16.0:
                continue
            r_end = (px1 - fold - 8.0) if ly < py0 + fold + 4.0 else margin_r
            d_r = sdf_rrect(x, y, margin_l, ly - rule_h * 0.5, r_end, ly + rule_h * 0.5, 0.2)
            d_rules = np.minimum(d_rules, d_r)
        canvas = over(canvas, rule_rgb, aa_mask(d_rules, px) * m_page * 0.24)

    # 2. Small-to-medium sizes (24–48 px): crisp simplified typographic bars (never mushy micro-text)
    if size <= 48:
        d_l1 = sdf_rrect(x, y, px0 + 16.0, py0 + 36.0, px1 - fold - 10.0, py0 + 44.0, 4.0)
        if highlight_first:
            d_hl = sdf_rrect(x, y, px0 + 13.0, py0 + 51.0, px1 - 24.0, py0 + 67.0, 5.0)
            canvas = over(canvas, (254, 214, 102), aa_mask(d_hl, px) * m_page * 0.72)
        d_l2 = sdf_rrect(x, y, px0 + 16.0, py0 + 55.0, px1 - 26.0, py0 + 63.0, 4.0)
        d_l3 = sdf_rrect(x, y, px0 + 16.0, py0 + 74.0, px1 - 52.0, py0 + 82.0, 4.0)
        canvas = over(canvas, (156, 166, 184), aa_mask(d_l1, px) * m_page * 0.85)
        canvas = over(canvas, (124, 134, 152), aa_mask(d_l2, px) * m_page * 0.88)
        canvas = over(canvas, (176, 184, 198), aa_mask(d_l3, px) * m_page * 0.80)
        return canvas

    # 3. Sizes >= 64 px: genuine inscribed editorial typography!
    # Verbatim quote ("In the Beginning, God created...") placed on the upper-mid rules,
    # staying cleanly clear of both the top-right corner fold and the fountain pen nib.
    page_w = px1 - px0
    text_x = px0 + 13.5
    max_w = page_w - 28.0

    base_pt = page_w * 0.104
    font_px = max(8, round(base_pt * scale))
    font = load_font(font_file, font_px)

    probe = ImageDraw.Draw(Image.new("L", (4, 4), 0))
    while font_px > 7:
        bbox = probe.textbbox((0, 0), "In the Beginning,", font=font)
        if (bbox[2] - bbox[0]) <= max_w * scale:
            break
        font_px -= 1
        font = load_font(font_file, font_px)

    line1_baseline = py0 + 68.0
    line2_baseline = py0 + 92.0
    if highlight_first:
        b1 = probe.textbbox((0, 0), "In the Beginning,", font=font)
        w1_u = (b1[2] - b1[0]) / scale
        hl_y0 = line1_baseline - (font_px / scale) * 0.90
        hl_y1 = line1_baseline + (font_px / scale) * 0.24
        d_hl = sdf_rrect(x, y, text_x - 3.5, hl_y0, text_x + w1_u + 4.5, hl_y1, 4.5)
        canvas = over(canvas, (254, 214, 102), aa_mask(d_hl, px) * m_page * 0.56)

    txt_mask = Image.new("L", (px, px), 0)
    td = ImageDraw.Draw(txt_mask)

    # Quiet chapter numeral on the top rule (clear of fold)
    orn_font = load_font(font_file, max(6, round(font_px * 0.64)))
    td.text(
        (text_x * scale, (py0 + 44.0 - 2.0) * scale),
        "I.",
        font=orn_font,
        fill=148,
        anchor="ls",
    )

    # Primary inscribed lines (verbatim quote, clean and unobstructed)
    td.text(
        (text_x * scale, (line1_baseline - 2.2) * scale),
        "In the Beginning,",
        font=font,
        fill=244,
        anchor="ls",
    )
    td.text(
        (text_x * scale, (line2_baseline - 2.2) * scale),
        "God created...",
        font=font,
        fill=238,
        anchor="ls",
    )

    alpha = (np.asarray(txt_mask, dtype=np.float32) / 255.0) * m_page
    return over(canvas, ink_rgb, alpha)


# ===========================================================================
# INSCRIBED VARIANT A: "CLASSIC INDIGO (INSCRIBED)"
# Open Fluent silhouette + Palatino Italic inscription + royal-indigo nib
# ===========================================================================

def draw_inscribed_classic(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    if not small:
        d_back = sdf_rot_rrect(x, y, 105.0, 122.0, 144.0, 192.0, 16.0, -5.5)
        m_back = aa_mask(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -1.0, 5.0, 8.0, 0.16, (18, 24, 52))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (220, 232, 254), (186, 204, 244))
        canvas = over(canvas, back_col, m_back)
        m_be = aa_mask(np.maximum(d_back, -(d_back + 2.2)), px)
        canvas = over(canvas, (124, 146, 204), m_be * 0.62)

    if small:
        px0, py0, px1, py1, pr, fold = 22.0, 14.0, 190.0, 236.0, 20.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 34.0, 22.0, 194.0, 232.0, 16.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 42.0, 24.0, 196.0, 228.0, 15.0, 36.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 1.0, 6.0, 9.0, 0.24, (16, 22, 48))

    paper = lin_grad(x, y, (px0, py0), (px1, py1), (255, 254, 249), (242, 239, 230))
    canvas = over(canvas, paper, m_page)

    # Inscribed Palatino Italic text & fine ruled lines
    canvas = composite_inscribed_text(
        canvas, x, y, px, px0, py0, px1, py1, fold, m_page, size,
        font_file="palai.ttf",
        ink_rgb=(42, 49, 74),
        rule_rgb=(130, 148, 186),
        highlight_first=False,
    )

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
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (226, 224, 215), (202, 200, 192)), m_flap)
    canvas = over(canvas, edge_col, aa_mask(np.maximum(d_flap, -(d_flap + edge_w * 0.8)), px) * m_page * 0.70)

    # Fluid calligraphic ink stroke
    tip_x, tip_y = (126.0, 176.0) if small else (140.0, 174.0)
    canvas = draw_calligraphic_stroke(
        canvas, x, y, px,
        p0=(58.0, 198.0) if not small else (46.0, 204.0),
        p1=(86.0, 208.0),
        p2=(110.0, 180.0),
        p3=(tip_x, tip_y),
        max_r=11.0 if small else 7.4,
        c0=(56, 78, 224),
        c1=(94, 58, 216),
        clip_mask=m_page,
    )

    # Sculpted Royal-Indigo Nib + Dual-Ring Satin Brass Ferrule
    canvas = draw_sculpted_nib(
        canvas, x, y, px,
        tip_x=tip_x, tip_y=tip_y, angle_deg=-42.0,
        scale=1.14 if small else 0.96,
        small=small, medium=medium,
        left_c0=(104, 94, 242), left_c1=(54, 44, 176),
        right_c0=(44, 34, 148), right_c1=(20, 15, 82),
        rim_col=(206, 214, 255), rim_alpha=0.45,
        collar_c0=(254, 222, 118), collar_c1=(184, 120, 20),
        band_col=(112, 66, 8),
        slit_c0=(255, 236, 154), slit_c1=(238, 172, 42),
        tip_glint=(255, 244, 200),
    )

    return downsample(canvas, size)


# ===========================================================================
# INSCRIBED VARIANT B: "GILDED TILE (INSCRIBED)"
# Framed midnight-cobalt squircle + upright Palatino book serif + 18k gold nib
# ===========================================================================

def draw_inscribed_gilded(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    r_tile = 118.0 if small else (114.0 if medium else 110.0)
    d_tile = sdf_superellipse(x, y, 128.0, 128.0, r_tile, r_tile, n=5.0)
    m_tile = aa_mask(d_tile, px)

    if not small and not medium:
        canvas = add_shadow(canvas, m_tile, px, 0.0, 5.0, 8.0, 0.30, (6, 12, 34))

    bg = lin_grad(x, y, (28, 18), (228, 242), (34, 68, 168), (13, 24, 62))
    glow = np.exp(-(((x - 84.0) / 120.0) ** 2 + ((y - 52.0) / 92.0) ** 2))[..., None]
    bg = np.clip(bg + glow * np.array([0.06, 0.09, 0.15], dtype=np.float32), 0.0, 1.0)
    canvas = over(canvas, bg, m_tile)

    rim_w = 6.5 if small else (4.2 if medium else 2.6)
    m_rim = aa_mask(np.maximum(d_tile, -(d_tile + rim_w)), px)
    rim_col = lin_grad(x, y, (128, 14), (128, 242), (160, 192, 255), (54, 76, 134))
    canvas = over(canvas, rim_col, m_rim * (0.78 if small else 0.62))

    if not small:
        d_back = sdf_rot_rrect(x, y, 100.0, 128.0, 110.0, 146.0, 13.0, -6.5)
        m_back = aa_mask(d_back, px) * m_tile
        canvas = over(canvas, lin_grad(x, y, (44, 52), (156, 198), (176, 204, 255), (96, 134, 224)), m_back * 0.32)

    if small:
        px0, py0, px1, py1, pr, fold = 42.0, 34.0, 182.0, 220.0, 15.0, 38.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 48.0, 38.0, 180.0, 214.0, 13.0, 36.0
    else:
        px0, py0, px1, py1, pr, fold = 50.0, 38.0, 182.0, 214.0, 13.0, 33.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 0.0, 6.5, 9.5, 0.40, (6, 12, 38), clip_mask=m_tile)

    canvas = over(canvas, lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 251), (241, 240, 234)), m_page)

    # Inscribed upright Palatino Book Serif & fine blue rules
    canvas = composite_inscribed_text(
        canvas, x, y, px, px0, py0, px1, py1, fold, m_page, size,
        font_file="pala.ttf",
        ink_rgb=(32, 44, 78),
        rule_rgb=(122, 146, 196),
        highlight_first=False,
    )

    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 4.5))
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.22, (12, 24, 64), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (218, 228, 246), (186, 200, 230)), m_flap)

    tip_x, tip_y = (124.0, 174.0) if small else (142.0, 182.0)
    canvas = draw_calligraphic_stroke(
        canvas, x, y, px,
        p0=(62.0, 196.0) if not small else (56.0, 196.0),
        p1=(88.0, 204.0),
        p2=(114.0, 188.0),
        p3=(tip_x, tip_y),
        max_r=10.5 if small else 6.6,
        c0=(26, 76, 204),
        c1=(54, 118, 246),
        clip_mask=m_page,
    )

    canvas = draw_sculpted_nib(
        canvas, x, y, px,
        tip_x=tip_x, tip_y=tip_y, angle_deg=-42.0,
        scale=1.02 if small else 0.80,
        small=small, medium=medium,
        left_c0=(255, 226, 122), left_c1=(232, 168, 36),
        right_c0=(212, 146, 24), right_c1=(154, 94, 12),
        rim_col=(255, 250, 218), rim_alpha=0.65,
        collar_c0=(56, 86, 176), collar_c1=(16, 28, 72),
        band_col=(180, 208, 255),
        slit_c0=(26, 46, 118), slit_c1=(12, 22, 64),
        tip_glint=(255, 252, 236),
        shadow_clip=m_tile,
    )

    return downsample(canvas, size)


# ===========================================================================
# INSCRIBED VARIANT C: "ATELIER TEAL (INSCRIBED)"
# Open silhouette + Palatino Italic with amber highlight wash + obsidian nib + emerald ink
# ===========================================================================

def draw_inscribed_atelier(size: int) -> Image.Image:
    ss = render_ss(size)
    px = size * ss
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    if not small:
        d_back = sdf_rot_rrect(x, y, 105.0, 122.0, 144.0, 192.0, 16.0, -5.8)
        m_back = aa_mask(d_back, px)
        if not medium:
            canvas = add_shadow(canvas, m_back, px, -1.0, 5.0, 8.0, 0.15, (14, 34, 30))
        back_col = lin_grad(x, y, (30, 20), (180, 220), (212, 240, 230), (170, 218, 204))
        canvas = over(canvas, back_col, m_back)
        m_be = aa_mask(np.maximum(d_back, -(d_back + 2.2)), px)
        canvas = over(canvas, (100, 162, 144), m_be * 0.65)

    if small:
        px0, py0, px1, py1, pr, fold = 22.0, 14.0, 190.0, 236.0, 20.0, 44.0
    elif medium:
        px0, py0, px1, py1, pr, fold = 34.0, 22.0, 194.0, 232.0, 16.0, 40.0
    else:
        px0, py0, px1, py1, pr, fold = 42.0, 24.0, 196.0, 228.0, 15.0, 36.0

    d_cut = sdf_halfplane(x, y, px1 - fold, py0, 1.0, -1.0)
    d_page = np.maximum(sdf_rrect(x, y, px0, py0, px1, py1, pr), d_cut)
    m_page = aa_mask(d_page, px)

    if not small:
        canvas = add_shadow(canvas, m_page, px, 1.0, 6.0, 9.0, 0.24, (14, 24, 28))

    canvas = over(canvas, lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 251), (242, 243, 237)), m_page)

    # Inscribed Palatino Italic + delicate warm-amber highlight wash on "In the Beginning,"
    canvas = composite_inscribed_text(
        canvas, x, y, px, px0, py0, px1, py1, fold, m_page, size,
        font_file="palai.ttf",
        ink_rgb=(32, 46, 48),
        rule_rgb=(118, 152, 144),
        highlight_first=True,
    )

    edge_w = 9.0 if small else (5.0 if medium else 2.4)
    edge_col = (72, 90, 96) if (small or medium) else (128, 142, 146)
    m_edge = aa_mask(np.maximum(d_page, -(d_page + edge_w)), px)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.94 if medium else 0.78)))

    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 5.0))
    m_flap = aa_mask(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -2.0, 3.5, 4.0, 0.18, (16, 30, 32), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (222, 230, 226), (196, 208, 204)), m_flap)
    canvas = over(canvas, edge_col, aa_mask(np.maximum(d_flap, -(d_flap + edge_w * 0.8)), px) * m_page * 0.70)

    tip_x, tip_y = (126.0, 176.0) if small else (140.0, 174.0)
    canvas = draw_calligraphic_stroke(
        canvas, x, y, px,
        p0=(58.0, 198.0) if not small else (46.0, 204.0),
        p1=(86.0, 208.0),
        p2=(110.0, 180.0),
        p3=(tip_x, tip_y),
        max_r=11.0 if small else 7.4,
        c0=(16, 168, 120),
        c1=(10, 118, 106),
        clip_mask=m_page,
    )

    canvas = draw_sculpted_nib(
        canvas, x, y, px,
        tip_x=tip_x, tip_y=tip_y, angle_deg=-42.0,
        scale=1.12 if small else 0.96,
        small=small, medium=medium,
        left_c0=(72, 86, 108), left_c1=(36, 44, 58),
        right_c0=(30, 38, 50), right_c1=(14, 18, 26),
        rim_col=(188, 204, 224), rim_alpha=0.42,
        collar_c0=(246, 186, 102), collar_c1=(168, 98, 28),
        band_col=(84, 44, 10),
        slit_c0=(252, 214, 134), slit_c1=(218, 146, 48),
        tip_glint=(234, 244, 248),
        elongated=True,
    )

    return downsample(canvas, size)


# ===========================================================================
# Comparison Plate (with 1024px detail loupe of the inscription!)
# ===========================================================================

def build_inscribed_plate(
    variants: list[tuple[str, str, str, dict[int, Image.Image]]],
    out_path: Path,
) -> None:
    W, H = 2640, 1880
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

    d.text((84, 56), 'EcclesiaPDF — Nib & Leaf: Inscribed Edition', font=f_title, fill=ink)
    d.text(
        (84, 114),
        'TextEdit-inspired manuscript inscription ("In the Beginning, God created...") in Palatino on fine ruled paper; clean fallback at <= 48 px',
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

        # Hero 420px render (slightly larger so the inscription reads effortlessly!)
        hero_box = [cx0, 286, cx0 + col_w, 756]
        d.rounded_rectangle(hero_box, radius=18, fill=(252, 252, 250, 255), outline=rule, width=1)
        big = icons[1024].resize((410, 410), Image.LANCZOS)
        img.alpha_composite(big, (cx_mid - 205, 316))

        d.text((cx0, 776), rationale, font=f_body, fill=ink)

        # True-size lineup on light
        d.text((cx0, 852), "TRUE SIZE ON LIGHT", font=f_mono, fill=muted)
        tx = cx0 + 16
        base_y = 1018
        for sz in [128, 64, 48, 32, 24, 16]:
            img.alpha_composite(icons[sz], (tx, base_y - sz))
            d.text((tx, base_y + 10), f"{sz}", font=f_mono, fill=muted)
            tx += sz + 36

        # Dark taskbar strip (#202022)
        bar_y0, bar_y1 = 1078, 1186
        d.rounded_rectangle([cx0, bar_y0, cx0 + col_w, bar_y1], radius=12, fill=(32, 32, 34, 255))
        tx = cx0 + 28
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, (bar_y0 + bar_y1 - sz) // 2))
            tx += sz + 44
        d.text((cx0, bar_y1 + 10), "DARK TASKBAR (#202022)", font=f_mono, fill=muted)

        # Light taskbar strip (#F3F3F3)
        lbar_y0, lbar_y1 = 1230, 1330
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

        # Detail loupe (cropped from 1024px) + 32px pixel grid side by side
        d.text((cx0, 1396), "INSCRIPTION DETAIL (1024 PX CROP)   /   32 PX GRID", font=f_mono, fill=muted)
        gy = 1430
        # Crop the upper-mid page where the inscription sits
        crop = icons[1024].crop((170, 210, 770, 680)).resize((360, 282), Image.LANCZOS)
        d.rounded_rectangle([cx0, gy, cx0 + 360, gy + 282], radius=10, fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(crop, (cx0, gy))
        d.text((cx0, gy + 292), "ACTUAL 1024 PX TYPE", font=f_mono, fill=muted)

        # 32px grid right beside it
        gx = cx0 + 392
        plate32 = icons[32].resize((256, 256), Image.NEAREST)
        d.rectangle([gx, gy + 13, gx + 256, gy + 269], fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(plate32, (gx, gy + 13))
        for k in range(33):
            d.line([(gx + k * 8, gy + 13), (gx + k * 8, gy + 269)], fill=(230, 228, 222, 255), width=1)
            d.line([(gx, gy + 13 + k * 8), (gx + 256, gy + 13 + k * 8)], fill=(230, 228, 222, 255), width=1)
        d.text((gx, gy + 292), "32x32  (8x)", font=f_mono, fill=muted)

    img.convert("RGB").save(out_path, optimize=True)


def main() -> None:
    specs: list[tuple[str, str, str, str, Callable[[int], Image.Image]]] = [
        (
            "classic_indigo",
            "A. Classic Indigo (Inscribed)",
            "Palatino Italic on ruled ivory, indigo nib & brass collar",
            "Warm manuscript feel: calligraphic italic inscription on\nfaint blue-grey rules, paired with the royal-indigo nib.",
            draw_inscribed_classic,
        ),
        (
            "gilded_tile",
            "B. Gilded Tile (Inscribed)",
            "Upright Palatino book serif, cobalt squircle & 18k gold nib",
            "Crisp typeset book feel inside a midnight-cobalt tile:\nupright Palatino text with a gleaming 18k gold nib.",
            draw_inscribed_gilded,
        ),
        (
            "atelier_teal",
            "C. Atelier Teal (Inscribed)",
            "Palatino Italic + warm highlight wash, obsidian nib & teal ink",
            "Annotated study manuscript: soft amber highlight over\n\"In the Beginning,\" with gunmetal nib and emerald ink.",
            draw_inscribed_atelier,
        ),
    ]

    rendered = []
    for slug, label, sub, rationale, fn in specs:
        out_dir = OUT_ROOT / slug
        out_dir.mkdir(parents=True, exist_ok=True)
        icons = {sz: fn(sz) for sz in SIZES}
        for sz in SIZES:
            icons[sz].save(out_dir / f"ecclesiapdf-{slug}-inscribed-{sz}.png", optimize=True)
        rendered.append((label, sub, rationale, icons))
        print(f"rendered {slug} ({len(SIZES)} sizes)")

    plate_path = OUT_ROOT / "nib-inscribed-sheet.png"
    build_inscribed_plate(rendered, plate_path)
    print("wrote", plate_path)


if __name__ == "__main__":
    main()
