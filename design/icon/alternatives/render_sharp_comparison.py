"""Renders a much sharper, high-precision edition of Nib & Leaf (Classic Indigo, Inscribed)
and builds a direct side-by-side comparison sheet against the current active render.

    python design/icon/alternatives/render_sharp_comparison.py

Does NOT overwrite design/icon/render_icon.py or packaging/windows/ecclesiapdf.ico.
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

import render_icon as current_icon  # noqa: E402
from render_icon import (  # noqa: E402
    PNG_SIZES,
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

OUT_DIR = HERE / "nib_sharp"


def aa_sharp(sdf: np.ndarray, px: int, sharpness: float = 0.68) -> np.ndarray:
    """Tighter analytic anti-aliasing transition for razor-crisp edges."""
    w = (256.0 / px) * sharpness
    return np.clip(0.5 - sdf / w, 0.0, 1.0)


def draw_sharp(size: int) -> Image.Image:
    # Always supersample >= 2x up to 512 px for razor-sharp serifs, hairlines and nib ridges
    ss = 8 if size <= 32 else (4 if size <= 128 else (2 if size <= 512 else 1))
    px = size * ss
    scale = px / 256.0
    x, y = make_grid(px)
    canvas = np.zeros((px, px, 4), dtype=np.float32)
    small = size <= 20
    medium = 20 < size <= 32

    # 1. Crisp cool-periwinkle back sheet (tighter corners & crisp slate-blue rim)
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

    # 2. Hero crisp paper sheet (sharper architectural corners, crisp tight shadow & edge)
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
        # Tight, crisp contact shadow instead of diffuse cloud
        canvas = add_shadow(canvas, m_page, px, 0.8, 3.5, 4.2, 0.28, (12, 18, 44))

    paper = lin_grad(x, y, (px0, py0), (px1, py1), (255, 255, 252), (245, 243, 236))
    canvas = over(canvas, paper, m_page)

    # 3. Crisp ruled hairlines & laser-sharp high-contrast Palatino Italic inscription
    if not small:
        if size >= 40:
            margin_l, margin_r = px0 + 13.5, px1 - 13.5
            line_ys = [py0 + 62.0, py0 + 86.0, py0 + 110.0, py0 + 134.0, py0 + 158.0]
            d_rules = np.full_like(x, 1e6, dtype=np.float32)
            rule_h = max(0.50, 0.72 * (256.0 / px))
            for ly in line_ys:
                d_r = sdf_rrect(x, y, margin_l, ly - rule_h * 0.5, margin_r, ly + rule_h * 0.5, 0.1)
                d_rules = np.minimum(d_rules, d_r)
            canvas = over(canvas, (112, 134, 178), aa_sharp(d_rules, px, 0.75) * m_page * 0.32)

        if size <= 48:
            # Razor-sharp typographic bars for small icon grids
            d_l1 = sdf_rrect(x, y, px0 + 15.0, py0 + 44.0, px1 - 24.0, py0 + 53.0, 2.2)
            d_l2 = sdf_rrect(x, y, px0 + 15.0, py0 + 64.0, px1 - 42.0, py0 + 73.0, 2.2)
            canvas = over(canvas, (92, 106, 138), aa_sharp(d_l1, px) * m_page * 0.96)
            canvas = over(canvas, (116, 128, 156), aa_sharp(d_l2, px) * m_page * 0.92)
        else:
            page_w = px1 - px0
            text_x = px0 + 13.5
            max_w = page_w - 27.0
            font_px = max(8, round(page_w * 0.107 * scale))
            serif_candidates = ["palai.ttf", "georgiai.ttf", "Palatino.ttc", "LiberationSerif-Italic.ttf"]
            font = load_font(serif_candidates, font_px)

            line1_text = "\u201cIn the Beginning,"
            line2_text = "God created...\u201d"
            probe = ImageDraw.Draw(Image.new("L", (4, 4), 0))
            while font_px > 7:
                bbox = probe.textbbox((0, 0), line1_text, font=font)
                if (bbox[2] - bbox[0]) <= max_w * scale:
                    break
                font_px -= 1
                font = load_font(serif_candidates, font_px)

            txt_mask = Image.new("L", (px, px), 0)
            td = ImageDraw.Draw(txt_mask)
            td.text((text_x * scale, (py0 + 62.0 - 2.2) * scale), line1_text, font=font, fill=255, anchor="ls")
            td.text((text_x * scale, (py0 + 86.0 - 2.2) * scale), line2_text, font=font, fill=252, anchor="ls")

            attr_px = max(6, round(font_px * 0.67))
            attr_font = load_font(serif_candidates, attr_px)
            attr_max_w = 88.0 * scale
            while attr_px > 6:
                ab = probe.textbbox((0, 0), "\u2014 (KJV Genesis 1:1)", font=attr_font)
                if (ab[2] - ab[0]) <= attr_max_w:
                    break
                attr_px -= 1
                attr_font = load_font(serif_candidates, attr_px)
            td.text((text_x * scale, (py0 + 110.0 - 2.2) * scale), "\u2014 (KJV Genesis 1:1)", font=attr_font, fill=208, anchor="ls")

            # Slight contrast gamma boost on the supersampled text mask for crisp letterpress bite
            raw_a = np.asarray(txt_mask, dtype=np.float32) / 255.0
            crisp_a = np.clip(raw_a ** 0.85, 0.0, 1.0) * m_page
            canvas = over(canvas, (22, 30, 56), crisp_a)

    # Crisp high-contrast page border (eliminates pale washed-out edge)
    edge_w = 9.5 if small else (5.2 if medium else 2.4)
    edge_col = (58, 72, 114) if (small or medium) else (92, 108, 146)
    m_edge = aa_sharp(np.maximum(d_page, -(d_page + edge_w)), px)
    canvas = over(canvas, edge_col, m_edge * (1.0 if small else (0.98 if medium else 0.92)))

    # Crisp folded top-right corner (sharper 90-deg inner corner + tighter crease shadow)
    flap_v = [(px1 - fold, py0), (px1, py0 + fold), (px1 - fold, py0 + fold)]
    d_flap = np.maximum(sdf_polygon(x, y, flap_v), sdf_rrect(x, y, px1 - fold, py0 - 8.0, px1 + 8.0, py0 + fold, 2.0))
    m_flap = aa_sharp(d_flap, px) * m_page
    if not small:
        canvas = add_shadow(canvas, m_flap, px, -1.6, 2.2, 2.4, 0.24, (16, 24, 54), clip_mask=m_page)
    canvas = over(canvas, lin_grad(x, y, (px1 - fold, py0 + fold), (px1, py0), (222, 226, 236), (190, 198, 218)), m_flap)
    canvas = over(canvas, edge_col, aa_sharp(np.maximum(d_flap, -(d_flap + edge_w * 0.85)), px) * m_page * 0.88)

    # 4. Crisp vivid calligraphic ink flourish
    tip_x, tip_y = (126.0, 176.0) if small else (140.0, 174.0)
    p0 = (46.0, 204.0) if small else (58.0, 198.0)
    p1, p2, p3 = (86.0, 208.0), (110.0, 180.0), (tip_x, tip_y)
    max_r = 11.2 if small else 7.4
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

    # 5. Razor-sharp faceted Royal-Indigo Fountain Nib + Precision Dual-Ring Brass Ferrule
    nib_sc = 1.14 if small else 0.96
    ang = math.radians(-42.0)
    ca, sa = math.cos(ang), math.sin(ang)
    dx, dy = x - tip_x, y - tip_y
    u = dx * ca + dy * sa
    v = -dx * sa + dy * ca

    # Slightly crisper, more authoritative architectural shoulders tapering to a needle-sharp tip
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
        # Tighter, higher-definition nib shadow
        canvas = add_shadow(canvas, m_pen_all, px, 1.8, 4.8, 3.8, 0.34, (10, 14, 38))

    # High-contrast faceted royal-indigo nib
    m_nib = aa_sharp(d_nib, px)
    left_facet = lin_grad(u, v, (0, -30 * nib_sc), (100 * nib_sc, 10 * nib_sc), (110, 102, 252), (48, 40, 182))
    right_facet = lin_grad(u, v, (0, 0), (100 * nib_sc, 32 * nib_sc), (36, 26, 138), (14, 10, 66))
    ridge = aa_sharp(v, px, 0.55)[..., None]
    canvas = over(canvas, left_facet * ridge + right_facet * (1.0 - ridge), m_nib)

    # Crisp specular edge bevel on lit upper flank & fine metallic center ridge highlight above breather hole
    if not small:
        m_nib_rim = aa_sharp(np.maximum(d_nib, -(d_nib + 1.8 * nib_sc)), px) * (v < 0).astype(np.float32)
        canvas = over(canvas, (224, 232, 255), m_nib_rim * 0.65)
        # Crisp dark outer rim on shaded lower flank so the nib edge snaps cleanly
        m_nib_dark = aa_sharp(np.maximum(d_nib, -(d_nib + 1.5 * nib_sc)), px) * (v >= 0).astype(np.float32)
        canvas = over(canvas, (12, 10, 48), m_nib_dark * 0.55)
        # Razor-fine center ridge specular glint between breather hole and collar
        d_ridge_glint = sdf_segment(u, v, 56.0 * nib_sc, -0.5 * nib_sc, base_u - 1.0 * nib_sc, -0.5 * nib_sc) - 0.7 * nib_sc
        canvas = over(canvas, (186, 194, 255), aa_sharp(d_ridge_glint, px) * m_nib * 0.36)

    # Crisp machined satin brass ferrule with two fine dark engraved grooves + bright metallic specular band
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

    # Luminous high-contrast gold breather hole & central slit
    hole_u = 49.0 * nib_sc
    hole_r = (8.8 if small else (7.2 if medium else 6.2)) * nib_sc
    slit_w = (2.6 if small else (1.95 if medium else 1.4)) * nib_sc
    d_slit = np.minimum(np.hypot(u - hole_u, v) - hole_r, sdf_segment(u, v, 8.0 * nib_sc, 0.0, hole_u, 0.0) - slit_w)
    slit_rgb = lin_grad(u, v, (10 * nib_sc, -8 * nib_sc), (58 * nib_sc, 8 * nib_sc), (255, 240, 160), (242, 174, 34))
    canvas = over(canvas, slit_rgb, aa_sharp(d_slit, px) * m_nib)

    # Bright crisp iridium tipping glint
    if not small and not medium:
        d_tip = np.hypot(u - 3.4 * nib_sc, v) - 2.1 * nib_sc
        canvas = over(canvas, (255, 248, 214), aa_sharp(d_tip, px) * m_nib * 0.92)

    img = downsample(canvas, size)
    # Subtle micro-unsharp finish on large display sizes (>= 128 px) without introducing halos
    if size >= 128:
        rgb = img.convert("RGB").filter(ImageFilter.UnsharpMask(radius=0.8, percent=55, threshold=2))
        a = img.getchannel("A")
        img = Image.merge("RGBA", (*rgb.split(), a))
    return img


def build_side_by_side(cur_icons: dict[int, Image.Image], sharp_icons: dict[int, Image.Image], out_path: Path) -> None:
    W, H = 2200, 1680
    bg = (246, 245, 241, 255)
    ink = (22, 28, 42, 255)
    muted = (92, 100, 114, 255)
    rule = (214, 211, 202, 255)

    img = Image.new("RGBA", (W, H), bg)
    d = ImageDraw.Draw(img)
    f_title = load_font(["segoeuib.ttf", "segoeui.ttf"], 42)
    f_sub = load_font(["segoeui.ttf"], 22)
    f_h2 = load_font(["segoeuib.ttf", "segoeui.ttf"], 32)
    f_mono = load_font(["consola.ttf"], 18)

    d.text((84, 52), "Nib & Leaf (Classic Indigo) — Current vs. Razor-Sharp Variant", font=f_title, fill=ink)
    d.text(
        (84, 108),
        "Tighter corners & shadows, higher-contrast page edge, 2x supersampled Palatino letterpress text, crisp nib ridge & dual-groove ferrule",
        font=f_sub,
        fill=muted,
    )
    d.line([(84, 158), (W - 84, 158)], fill=rule, width=2)

    cols = [
        ("Current (Soft)", "15u corners, diffuse shadows, softer grey rim", cur_icons),
        ("Sharp Edition", "8u corners, tight shadows, crisp ink & metallic bevels", sharp_icons),
    ]
    col_w = (W - 168 - 64) // 2
    for idx, (title_txt, sub_txt, icons) in enumerate(cols):
        cx0 = 84 + idx * (col_w + 64)
        cx_mid = cx0 + col_w // 2
        if idx == 1:
            d.line([(cx0 - 32, 186), (cx0 - 32, H - 64)], fill=rule, width=1)

        d.text((cx0, 184), title_txt, font=f_h2, fill=ink)
        d.text((cx0, 226), sub_txt, font=f_sub, fill=muted)

        # 460px hero preview
        d.rounded_rectangle([cx0, 270, cx0 + col_w, 770], radius=16, fill=(253, 253, 251, 255), outline=rule, width=1)
        hero = icons[512].resize((450, 450), Image.LANCZOS)
        img.alpha_composite(hero, (cx_mid - 225, 295))

        # True-size row on light
        d.text((cx0, 796), "TRUE SIZE ON LIGHT (128 -> 16 PX)", font=f_mono, fill=muted)
        tx, base_y = cx0 + 20, 962
        for sz in [128, 64, 48, 32, 24, 16]:
            img.alpha_composite(icons[sz], (tx, base_y - sz))
            d.text((tx, base_y + 10), f"{sz}", font=f_mono, fill=muted)
            tx += sz + 44

        # Dark & light taskbar strips
        d.rounded_rectangle([cx0, 1020, cx0 + col_w, 1120], radius=10, fill=(32, 32, 34, 255))
        tx = cx0 + 36
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, 1020 + (100 - sz) // 2))
            tx += sz + 56
        d.text((cx0, 1130), "DARK TASKBAR (#202022)", font=f_mono, fill=muted)

        d.rounded_rectangle([cx0, 1170, cx0 + col_w, 1264], radius=10, fill=(243, 243, 243, 255), outline=(220, 220, 220, 255), width=1)
        tx = cx0 + 36
        for sz in [64, 48, 40, 32, 24, 20, 16]:
            img.alpha_composite(icons[sz], (tx, 1170 + (94 - sz) // 2))
            tx += sz + 56
        d.text((cx0, 1274), "LIGHT TASKBAR (#F3F3F3)", font=f_mono, fill=muted)

        # 512px crispness crop (1:1 pixels zoomed 1.5x) + 32px pixel grid
        d.text((cx0, 1328), "DETAIL CROP (512 PX)                 32 PX GRID (8x)", font=f_mono, fill=muted)
        gy = 1360
        crop = icons[512].crop((86, 112, 430, 376)).resize((410, 256), Image.NEAREST)
        d.rounded_rectangle([cx0, gy, cx0 + 410, gy + 256], radius=8, fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(crop, (cx0, gy))

        gx = cx0 + 448
        plate32 = icons[32].resize((256, 256), Image.NEAREST)
        d.rectangle([gx, gy, gx + 256, gy + 256], fill=(255, 255, 255, 255), outline=rule, width=1)
        img.alpha_composite(plate32, (gx, gy))
        for k in range(33):
            d.line([(gx + k * 8, gy), (gx + k * 8, gy + 256)], fill=(230, 228, 222, 255), width=1)
            d.line([(gx, gy + k * 8), (gx + 256, gy + k * 8)], fill=(230, 228, 222, 255), width=1)

    img.convert("RGB").save(out_path, optimize=True)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    sharp_icons = {sz: draw_sharp(sz) for sz in PNG_SIZES}
    for sz in PNG_SIZES:
        sharp_icons[sz].save(OUT_DIR / f"ecclesiapdf-sharp-{sz}.png", optimize=True)

    cur_icons = {sz: Image.open(ICON_DIR / "png" / f"ecclesiapdf-{sz}.png").convert("RGBA") for sz in PNG_SIZES}
    comp_path = OUT_DIR / "sharp-vs-current.png"
    build_side_by_side(cur_icons, sharp_icons, comp_path)
    print("wrote", comp_path)


if __name__ == "__main__":
    main()
