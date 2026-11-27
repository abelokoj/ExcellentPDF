"""Renders two new Sharp Nib & Leaf (Classic Indigo) variants with full scripture quotations:
  1. “¹ In the beginning God created the heaven and the earth.” (KJV Genesis 1:1)
  2. “¹³ I am the Alpha and the Omega, the Beginning and the End, the First and the Last.” (NKJV Revelation 22:13)

    python design/icon/alternatives/render_scripture_variants.py

Does NOT modify design/icon/render_icon.py or packaging/windows/ecclesiapdf.ico.
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
ICON_DIR = HERE.parent
sys.path.insert(0, str(ICON_DIR))

from render_icon import (  # noqa: E402
    PNG_SIZES,
    aa_sharp,
    add_shadow,
    downsample,
    lin_grad,
    load_font,
    make_grid,
    over,
    sample_cubic,
    sdf_halfplane,
    sdf_polygon,
    sdf_rot_rrect,
    sdf_rrect,
    sdf_segment,
)

OUT_ROOT = HERE / "nib_scripture"
SERIF_CANDIDATES = ["palai.ttf", "georgiai.ttf", "Palatino.ttc", "LiberationSerif-Italic.ttf"]


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
    """Draws opening quote + true typographic superscript verse number + thin space + text.
    Returns total advance width in pixels."""
    cx = x_px
    if open_quote:
        td.text((cx, baseline_px), open_quote, font=main_font, fill=fill, anchor="ls")
        cx += td.textlength(open_quote, font=main_font)
    if verse_num:
        # Lift superscript by ~34% of font cap height
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


def draw_scripture_icon(
    size: int,
    verse_num: str,
    lines: list[tuple[str, float]],
    attribution: str,
    line_ys: list[float],
    base_pt_ratio: float,
    attr_ratio: float,
    attr_max_w_u: float,
) -> Image.Image:
    """Sharp Classic Indigo Nib & Leaf renderer parameterized by multi-line scripture block.
    `lines` is a list of `(text, max_width_units)` where line 0 automatically receives
    the opening curly quote + true Palatino superscript `verse_num`."""
    ss = 8 if size <= 32 else (4 if size <= 128 else (2 if size <= 512 else 1))
    px = size * ss
    scale = px / 256.0
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Crisp cool-periwinkle back sheet
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

    # 3. Crisp visible ruled stationery lines & scripture typography
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
            # Clean simplified typographic bars for small icon grids
            d_l1 = sdf_rrect(x, y, px0 + 15.0, py0 + 40.0, px1 - 24.0, py0 + 48.5, 2.2)
            d_l2 = sdf_rrect(x, y, px0 + 15.0, py0 + 58.0, px1 - 28.0, py0 + 66.5, 2.2)
            d_l3 = sdf_rrect(x, y, px0 + 15.0, py0 + 76.0, px1 - 52.0, py0 + 84.5, 2.2)
            canvas = over(canvas, (92, 106, 138), aa_sharp(d_l1, px) * m_page * 0.96)
            canvas = over(canvas, (102, 116, 146), aa_sharp(d_l2, px) * m_page * 0.92)
            canvas = over(canvas, (124, 136, 162), aa_sharp(d_l3, px) * m_page * 0.88)
        else:
            page_w = px1 - px0
            text_x = px0 + 12.0
            font_px = max(7, round(page_w * base_pt_ratio * scale))
            probe = ImageDraw.Draw(Image.new("L", (4, 4), 0))

            # Fit every line cleanly within its per-line available width (stays clear of fold & nib)
            while font_px > 6:
                font = load_font(SERIF_CANDIDATES, font_px)
                sup_font = load_font(SERIF_CANDIDATES, max(5, round(font_px * 0.62)))
                fits = True
                for i, (txt, max_u) in enumerate(lines):
                    if i == 0:
                        w_px = measure_with_superscript(probe, "\u201c", verse_num, txt, font, sup_font)
                    else:
                        w_px = probe.textlength(txt, font=font)
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

            # Attribution line on the next ruled line
            attr_idx = len(lines)
            attr_px = max(6, round(font_px * attr_ratio))
            attr_font = load_font(SERIF_CANDIDATES, attr_px)
            while attr_px > 5:
                if probe.textlength(attribution, font=attr_font) <= attr_max_w_u * scale:
                    break
                attr_px -= 1
                attr_font = load_font(SERIF_CANDIDATES, attr_px)
            td.text(
                (text_x * scale, (py0 + line_ys[attr_idx] - 2.3) * scale),
                attribution,
                font=attr_font,
                fill=214,
                anchor="ls",
            )

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


def draw_genesis_full(size: int) -> Image.Image:
    """“¹ In the beginning God created the heaven and the earth.” — (KJV Genesis 1:1)"""
    return draw_scripture_icon(
        size=size,
        verse_num="1",
        lines=[
            ("In the beginning", 128.0),
            ("God created the heaven", 128.0),
            ("and the earth.\u201d", 104.0),
        ],
        attribution="\u2014 (KJV Genesis 1:1)",
        line_ys=[54.0, 72.0, 90.0, 108.0, 126.0, 144.0, 162.0],
        base_pt_ratio=0.088,
        attr_ratio=0.74,
        attr_max_w_u=90.0,
    )


def draw_revelation(size: int) -> Image.Image:
    """“¹³ I am the Alpha and the Omega, the Beginning and the End, the First and the Last.” — (NKJV Revelation 22:13)"""
    return draw_scripture_icon(
        size=size,
        verse_num="13",
        lines=[
            ("I am the Alpha and", 128.0),
            ("the Omega, the Beginning", 128.0),
            ("and the End, the First", 108.0),
            ("and the Last.\u201d", 94.0),
        ],
        attribution="\u2014 (NKJV Revelation 22:13)",
        line_ys=[52.0, 67.5, 83.0, 98.5, 114.0, 129.5, 145.0, 160.5],
        base_pt_ratio=0.076,
        attr_ratio=0.76,
        attr_max_w_u=90.0,
    )


def build_plate(
    v1_icons: dict[int, Image.Image],
    v2_icons: dict[int, Image.Image],
    out_path: Path,
) -> None:
    W, H = 2240, 1740
    bg = (246, 245, 241, 255)
    ink = (22, 28, 42, 255)
    muted = (92, 100, 114, 255)
    rule = (214, 211, 202, 255)

    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)
    f_title = load_font(["segoeuib.ttf", "segoeui.ttf"], 42)
    f_sub = load_font(["segoeui.ttf"], 22)
    f_h2 = load_font(["segoeuib.ttf", "segoeui.ttf"], 30)
    f_mono = load_font(["consola.ttf"], 18)

    d.text((84, 52), "Nib & Leaf (Sharp Classic Indigo) — Full Scripture Variants", font=f_title, fill=ink)
    d.text(
        (84, 108),
        "True Palatino superscript verse numbers, crisp stationery rules, clean clearance from corner fold & fountain nib (16 px to 1024 px)",
        font=f_sub,
        fill=muted,
    )
    d.line([(84, 158), (W - 84, 158)], fill=rule, width=2)

    cols = [
        (
            "Variant 1 — Genesis 1:1 (KJV)",
            '\u201c\u00b9 In the beginning God created the heaven and the earth.\u201d',
            v1_icons,
        ),
        (
            "Variant 2 — Revelation 22:13 (NKJV)",
            '\u201c\u00b9\u00b3 I am the Alpha and the Omega, the Beginning and the End, the First and the Last.\u201d',
            v2_icons,
        ),
    ]
    col_w = (W - 168 - 64) // 2
    for idx, (title_txt, quote_txt, icons) in enumerate(cols):
        cx0 = 84 + idx * (col_w + 64)
        cx_mid = cx0 + col_w // 2
        if idx == 1:
            d.line([(cx0 - 32, 186), (cx0 - 32, H - 64)], fill=rule, width=1)

        d.text((cx0, 184), title_txt, font=f_h2, fill=ink)
        # Keep subtitle neat on one line
        sub_font = load_font(["segoeui.ttf"], 19 if idx == 1 else 21)
        d.text((cx0, 228), quote_txt, font=sub_font, fill=muted)

        # 460px hero render
        d.rounded_rectangle([cx0, 272, cx0 + col_w, 772], radius=16, fill=(253, 253, 251, 255), outline=rule, width=1)
        hero = icons[1024].resize((460, 460), Image.LANCZOS)
        img.alpha_composite(hero, (cx_mid - 230, 292))

        # True-size row on light
        d.text((cx0, 798), "TRUE SIZE ON LIGHT (256 -> 16 PX)", font=f_mono, fill=muted)
        tx, base_y = cx0 + 16, 1020
        for sz in [128, 64, 48, 32, 24, 16]:
            img.alpha_composite(icons[sz], (tx, base_y - sz))
            d.text((tx, base_y + 10), f"{sz}", font=f_mono, fill=muted)
            tx += sz + 44

        # Dark & light taskbar strips
        d.rounded_rectangle([cx0, 1076, cx0 + col_w, 1176], radius=10, fill=(32, 32, 34, 255))
        tx = cx0 + 36
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, 1076 + (100 - sz) // 2))
            tx += sz + 56
        d.text((cx0, 1186), "DARK TASKBAR (#202022)", font=f_mono, fill=muted)

        d.rounded_rectangle([cx0, 1226, cx0 + col_w, 1320], radius=10, fill=(243, 243, 243, 255), outline=(220, 220, 220, 255), width=1)
        tx = cx0 + 36
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, 1226 + (94 - sz) // 2))
            tx += sz + 56
        d.text((cx0, 1330), "LIGHT TASKBAR (#F3F3F3)", font=f_mono, fill=muted)

        # 1024px inscription detail crop + 32px pixel grid
        d.text((cx0, 1384), "INSCRIPTION DETAIL (1024 PX CROP)       32 PX GRID (8x)", font=f_mono, fill=muted)
        gy = 1416
        crop = icons[1024].crop((176, 196, 820, 680)).resize((430, 256), Image.LANCZOS)
        d.rounded_rectangle([cx0, gy, cx0 + 430, gy + 256], radius=8, fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(crop, (cx0, gy))

        gx = cx0 + 468
        plate32 = icons[32].resize((256, 256), Image.NEAREST)
        d.rectangle([gx, gy, gx + 256, gy + 256], fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(plate32, (gx, gy))
        for k in range(33):
            d.line([(gx + k * 8, gy), (gx + k * 8, gy + 256)], fill=(230, 228, 222, 255), width=1)
            d.line([(gx, gy + k * 8), (gx + 256, gy + k * 8)], fill=(230, 228, 222, 255), width=1)

    img.convert("RGB").save(out_path, optimize=True)


def main() -> None:
    g_dir = OUT_ROOT / "genesis_1_1_kjv"
    r_dir = OUT_ROOT / "revelation_22_13_nkjv"
    g_dir.mkdir(parents=True, exist_ok=True)
    r_dir.mkdir(parents=True, exist_ok=True)

    g_icons = {sz: draw_genesis_full(sz) for sz in PNG_SIZES}
    r_icons = {sz: draw_revelation(sz) for sz in PNG_SIZES}
    for sz in PNG_SIZES:
        g_icons[sz].save(g_dir / f"ecclesiapdf-genesis-{sz}.png", optimize=True)
        r_icons[sz].save(r_dir / f"ecclesiapdf-revelation-{sz}.png", optimize=True)

    plate = OUT_ROOT / "scripture-variants-sheet.png"
    build_plate(g_icons, r_icons, plate)
    print("wrote", plate)


if __name__ == "__main__":
    main()
