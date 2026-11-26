"""Render monochrome white-on-transparent PNG UI icons for ExcellentPDF.

NSH 0.7 is built without its optional SVG rasterizer, while PNG data URIs
decode via WIC and tint cleanly through `tint_color(...)`. Each icon is drawn
at 4x resolution and LANCZOS-downsampled to crisp anti-aliased RGBA pixels.
"""

from pathlib import Path
from PIL import Image, ImageDraw

OUT_DIR = Path(__file__).resolve().parents[2] / "crates" / "excellent-app" / "assets" / "icons"
WHITE = (255, 255, 255, 255)
CLEAR = (255, 255, 255, 0)
S = 192  # 4x supersampling for 48x48 icons


def save_icon(name: str, canvas: Image.Image, target_size: tuple[int, int] = (48, 48)) -> Path:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out = canvas.resize(target_size, Image.Resampling.LANCZOS)
    path = OUT_DIR / f"{name}.png"
    out.save(path, format="PNG", optimize=True)
    return path


def draw_thick_line(draw: ImageDraw.ImageDraw, pts: list[tuple[int, int]], width: int) -> None:
    draw.line(pts, fill=WHITE, width=width, joint="curve")
    r = width / 2.0
    for x, y in pts:
        draw.ellipse((x - r, y - r, x + r, y + r), fill=WHITE)


def render_triangle_down() -> Image.Image:
    im = Image.new("RGBA", (128, 96), CLEAR)
    d = ImageDraw.Draw(im)
    d.polygon([(14, 24), (114, 24), (64, 76)], fill=WHITE)
    return im


def render_home() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 15
    # Pitched roof
    draw_thick_line(d, [(24, 86), (96, 26), (168, 86)], w)
    # Walls + base + doorway
    draw_thick_line(d, [(42, 76), (42, 160), (76, 160), (76, 114), (116, 114), (116, 160), (150, 160), (150, 76)], w)
    return im


def render_search() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 16
    d.ellipse((28, 28, 134, 134), outline=WHITE, width=w)
    draw_thick_line(d, [(124, 124), (164, 164)], w)
    return im


def render_palette() -> Image.Image:
    """Command symbol (⌘) matching Cmd/Ctrl+K."""
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 14
    r = 24
    # Outer loop corners
    # Top-left loop: (68,68) back-left to (20,68) up to (20,20) right to (68,20) down to (68,172)
    draw_thick_line(d, [(68, 68), (36, 68), (24, 56), (24, 36), (36, 24), (56, 24), (68, 36), (68, 156), (56, 168), (36, 168), (24, 156), (24, 136), (36, 124), (156, 124), (168, 136), (168, 156), (156, 168), (136, 168), (124, 156), (124, 36), (136, 24), (156, 24), (168, 36), (168, 56), (156, 68), (68, 68)], w)
    return im


def render_keyboard() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 14
    d.rounded_rectangle((16, 42, 176, 150), radius=20, outline=WHITE, width=w)
    for y in (72, 100):
        for x in (44, 78, 112, 146):
            d.ellipse((x - 7, y - 7, x + 7, y + 7), fill=WHITE)
    draw_thick_line(d, [(58, 126), (134, 126)], 12)
    return im


def render_folder() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 15
    draw_thick_line(
        d,
        [(24, 56), (24, 154), (168, 154), (168, 66), (96, 66), (78, 44), (36, 44), (24, 56)],
        w,
    )
    return im


def render_clock() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 15
    d.ellipse((24, 24, 168, 168), outline=WHITE, width=w)
    draw_thick_line(d, [(96, 54), (96, 98), (128, 118)], w)
    return im


def render_grid() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 14
    for box in [(26, 26, 82, 82), (110, 26, 166, 82), (26, 110, 82, 166), (110, 110, 166, 166)]:
        d.rounded_rectangle(box, radius=10, outline=WHITE, width=w)
    return im


def render_list() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 15
    for y in (48, 96, 144):
        d.ellipse((24, y - 9, 42, y + 9), fill=WHITE)
        draw_thick_line(d, [(66, y), (166, y)], w)
    return im


def render_zoom_out() -> Image.Image:
    im = render_search()
    d = ImageDraw.Draw(im)
    draw_thick_line(d, [(58, 81), (104, 81)], 14)
    return im


def render_zoom_in() -> Image.Image:
    im = render_search()
    d = ImageDraw.Draw(im)
    draw_thick_line(d, [(58, 81), (104, 81)], 14)
    draw_thick_line(d, [(81, 58), (81, 104)], 14)
    return im


def render_book() -> Image.Image:
    im = Image.new("RGBA", (S, S), CLEAR)
    d = ImageDraw.Draw(im)
    w = 14
    draw_thick_line(
        d,
        [(22, 46), (96, 62), (170, 46), (170, 146), (96, 162), (22, 146), (22, 46)],
        w,
    )
    draw_thick_line(d, [(96, 62), (96, 160)], w)
    return im


def main() -> None:
    specs = [
        ("triangle-down", render_triangle_down(), (32, 24)),
        ("home", render_home(), (48, 48)),
        ("search", render_search(), (48, 48)),
        ("palette", render_palette(), (48, 48)),
        ("keyboard", render_keyboard(), (48, 48)),
        ("folder", render_folder(), (48, 48)),
        ("clock", render_clock(), (48, 48)),
        ("grid", render_grid(), (48, 48)),
        ("list", render_list(), (48, 48)),
        ("zoom-out", render_zoom_out(), (48, 48)),
        ("zoom-in", render_zoom_in(), (48, 48)),
        ("book", render_book(), (48, 48)),
    ]
    total = 0
    for name, canvas, size in specs:
        p = save_icon(name, canvas, size)
        sz = p.stat().st_size
        total += sz
        alpha_non_zero = sum(1 for px in Image.open(p).getdata() if px[3] > 0)
        assert alpha_non_zero > 0, f"{name} has no non-transparent pixels"
        print(f"{p.name}: {sz} bytes, {alpha_non_zero} opaque/AA px")
    print(f"Total icons: {len(specs)}, total bytes: {total}")


if __name__ == "__main__":
    main()
