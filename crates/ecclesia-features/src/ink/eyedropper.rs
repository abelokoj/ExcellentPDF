//! Eyedropper color picker (H.17, plan §27.7): samples the exact RGB color from a rendered page or annotation pixmap
//! *before* dark mode recoloring is applied, with optional 3×3 averaging and a magnifier loupe grid.

use ecclesia_mupdf::{Background, DeviceRect, DisplayList, Matrix, PagePoint, Pixmap, Result};

use crate::annotate::colors::{Colors, Tool};
use crate::ink::favorites::Favorites;

/// An exact 8-bit sRGB color sampled from a rendered pixmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SampledColor {
    /// Red channel (`0..=255`).
    pub r: u8,
    /// Green channel (`0..=255`).
    pub g: u8,
    /// Blue channel (`0..=255`).
    pub b: u8,
}

impl SampledColor {
    /// Constructs a color from 8-bit RGB channels.
    #[must_use]
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Constructs a color from `0xRRGGBB`.
    #[must_use]
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as u8,
            g: ((hex >> 8) & 0xff) as u8,
            b: (hex & 0xff) as u8,
        }
    }

    /// Packs the color into `0xRRGGBB`.
    #[must_use]
    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Formats the color as `#rrggbb`.
    #[must_use]
    pub fn to_hex_string(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Converts to normalized `0.0..=1.0` RGB channels for PDF annotations.
    #[must_use]
    pub fn to_rgb_f32(self) -> [f32; 3] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        ]
    }

    /// WCAG relative luminance (`0.0` black to `1.0` white), useful for choosing high-contrast loupe crosshair color.
    #[must_use]
    pub fn luminance(self) -> f32 {
        let [r, g, b] = self.to_rgb_f32().map(|c| {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

/// Pixel sampling aperture for the eyedropper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SampleArea {
    /// Exact single pixel at `(x, y)`.
    #[default]
    SinglePixel,
    /// Mean of the in-bounds 3×3 pixel neighborhood centered at `(x, y)`.
    Average3x3,
}

/// Alpha representation of an RGBA buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 3 bytes per pixel (`R, G, B`).
    Rgb,
    /// 4 bytes per pixel (`R, G, B, A`), premultiplied alpha over white (as MuPDF renders).
    PremultipliedRgba,
    /// 4 bytes per pixel (`R, G, B, A`), straight (unpremultiplied) alpha over white.
    StraightRgba,
}

/// Borrowed view over raw pre-dark-mode rendered pixels.
#[derive(Debug, Clone, Copy)]
pub struct PixelBuffer<'a> {
    /// Raw pixel bytes in row-major order.
    pub samples: &'a [u8],
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Byte stride per row.
    pub stride: usize,
    /// Channel layout.
    pub format: PixelFormat,
}

impl<'a> PixelBuffer<'a> {
    /// Wraps a rendered MuPDF [`Pixmap`] (before dark-mode recoloring).
    #[must_use]
    pub fn from_pixmap(pixmap: &'a Pixmap) -> Self {
        let format = if pixmap.has_alpha() || pixmap.components() >= 4 {
            PixelFormat::PremultipliedRgba
        } else {
            PixelFormat::Rgb
        };
        Self {
            samples: pixmap.samples(),
            width: pixmap.width(),
            height: pixmap.height(),
            stride: pixmap.stride(),
            format,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        match self.format {
            PixelFormat::Rgb => 3,
            PixelFormat::PremultipliedRgba | PixelFormat::StraightRgba => 4,
        }
    }

    /// Reads the exact pixel at `(x, y)`, compositing RGBA pixels over white paper.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<SampledColor> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let bpp = self.bytes_per_pixel();
        let offset = (y as usize)
            .checked_mul(self.stride)?
            .checked_add((x as usize).checked_mul(bpp)?)?;
        let slice = self.samples.get(offset..offset.checked_add(bpp)?)?;
        let (r, g, b) = match self.format {
            PixelFormat::Rgb => (slice[0], slice[1], slice[2]),
            PixelFormat::PremultipliedRgba => {
                let a = slice[3];
                let bg = 255u8.saturating_sub(a);
                (
                    slice[0].saturating_add(bg),
                    slice[1].saturating_add(bg),
                    slice[2].saturating_add(bg),
                )
            }
            PixelFormat::StraightRgba => {
                let a = u16::from(slice[3]);
                let inv = 255u16.saturating_sub(a);
                let mix = |c: u8| ((u16::from(c) * a + 255 * inv + 127) / 255) as u8;
                (mix(slice[0]), mix(slice[1]), mix(slice[2]))
            }
        };
        Some(SampledColor::from_rgb(r, g, b))
    }
}

/// Samples a color from `buffer` at pixel `(x, y)` using `area`; returns `None` when out of bounds.
#[must_use]
pub fn sample_pixel(
    buffer: &PixelBuffer<'_>,
    x: u32,
    y: u32,
    area: SampleArea,
) -> Option<SampledColor> {
    match area {
        SampleArea::SinglePixel => buffer.pixel(x, y),
        SampleArea::Average3x3 => {
            if x >= buffer.width || y >= buffer.height {
                return None;
            }
            let (mut sr, mut sg, mut sb, mut count) = (0u32, 0u32, 0u32, 0u32);
            let x0 = x.saturating_sub(1);
            let y0 = y.saturating_sub(1);
            let x1 = (x + 1).min(buffer.width.saturating_sub(1));
            let y1 = (y + 1).min(buffer.height.saturating_sub(1));
            for py in y0..=y1 {
                for px in x0..=x1 {
                    if let Some(c) = buffer.pixel(px, py) {
                        sr += u32::from(c.r);
                        sg += u32::from(c.g);
                        sb += u32::from(c.b);
                        count += 1;
                    }
                }
            }
            (count > 0).then(|| {
                SampledColor::from_rgb(
                    ((sr + count / 2) / count) as u8,
                    ((sg + count / 2) / count) as u8,
                    ((sb + count / 2) / count) as u8,
                )
            })
        }
    }
}

/// Samples a color directly from a pre-dark-mode MuPDF [`Pixmap`].
#[must_use]
pub fn sample_pixmap(pixmap: &Pixmap, x: u32, y: u32, area: SampleArea) -> Option<SampledColor> {
    sample_pixel(&PixelBuffer::from_pixmap(pixmap), x, y, area)
}

/// Samples the exact color at `page_point` directly from the page's [`DisplayList`] on white paper *without* dark-mode
/// recoloring, rendering only a tiny 1×1 (or 3×3) device patch.
///
/// # Errors
///
/// Fails if MuPDF cannot render the patch or if `scale` is non-positive.
pub fn sample_page_point(
    list: &DisplayList,
    page_point: PagePoint,
    scale: f32,
    area: SampleArea,
) -> Result<SampledColor> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let ctm = Matrix::scale(scale, scale);
    let dev_x = (page_point.x * scale).floor() as i32;
    let dev_y = (page_point.y * scale).floor() as i32;
    let pad = match area {
        SampleArea::SinglePixel => 0,
        SampleArea::Average3x3 => 1,
    };
    let patch = DeviceRect {
        x0: dev_x - pad,
        y0: dev_y - pad,
        x1: dev_x + pad + 1,
        y1: dev_y + pad + 1,
    };
    let pixmap = list.render_area(&ctm, patch, Background::White, None)?;
    let local = pad as u32;
    Ok(sample_pixmap(&pixmap, local, local, area).unwrap_or(SampledColor::from_rgb(255, 255, 255)))
}

/// Magnifier loupe grid extracted around the eyedropper cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoupeGrid {
    /// Grid side length (`2 * radius + 1`).
    pub side: u32,
    /// Pixels in row-major order (`None` for coordinates outside the pixmap).
    pub pixels: Vec<Option<SampledColor>>,
    /// The center pixel under the crosshair.
    pub center: Option<SampledColor>,
}

/// Extracts a `(2 * radius + 1)` square pixel grid centered at `(center_x, center_y)` for rendering a color loupe.
#[must_use]
pub fn loupe_grid(
    buffer: &PixelBuffer<'_>,
    center_x: u32,
    center_y: u32,
    radius: u32,
) -> LoupeGrid {
    let radius = radius.min(16);
    let side = radius * 2 + 1;
    let mut pixels = Vec::with_capacity((side * side) as usize);
    let cx = i64::from(center_x);
    let cy = i64::from(center_y);
    let r = i64::from(radius);

    for dy in -r..=r {
        for dx in -r..=r {
            let px = cx + dx;
            let py = cy + dy;
            let color = match (u32::try_from(px).ok(), u32::try_from(py).ok()) {
                (Some(x), Some(y)) => buffer.pixel(x, y),
                _ => None,
            };
            pixels.push(color);
        }
    }
    let center = buffer.pixel(center_x, center_y);
    LoupeGrid {
        side,
        pixels,
        center,
    }
}

/// Interactive state of the eyedropper tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EyedropperState {
    active: bool,
    /// Sampling aperture.
    pub area: SampleArea,
    /// Color currently under the cursor while active.
    pub hover: Option<SampledColor>,
    /// Most recently committed picked color.
    pub last_picked: Option<SampledColor>,
}

impl EyedropperState {
    /// Creates an inactive eyedropper with single-pixel precision.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the eyedropper tool is currently active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Activates the eyedropper tool.
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// Deactivates the eyedropper tool and clears live hover state.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.hover = None;
    }

    /// Updates the hovered color from a pre-dark-mode buffer.
    pub fn hover_at(&mut self, buffer: &PixelBuffer<'_>, x: u32, y: u32) -> Option<SampledColor> {
        self.hover = sample_pixel(buffer, x, y, self.area);
        self.hover
    }

    /// Commits a color pick from a pre-dark-mode buffer and deactivates the tool.
    pub fn pick_at(&mut self, buffer: &PixelBuffer<'_>, x: u32, y: u32) -> Option<SampledColor> {
        let color = sample_pixel(buffer, x, y, self.area)?;
        self.last_picked = Some(color);
        self.hover = Some(color);
        self.active = false;
        Some(color)
    }

    /// Applies `last_picked` to the active pen in `favorites`. Returns `true` when a color was applied.
    pub fn apply_to_favorites(&self, favorites: &mut Favorites) -> bool {
        let Some(picked) = self.last_picked else {
            return false;
        };
        let idx = favorites.chosen();
        let mut pen = favorites.in_hand();
        pen.color = picked.to_hex();
        favorites.set(idx, pen);
        true
    }

    /// Applies `last_picked` to `tool` in A.3 `colors`. Returns `true` when a color was applied.
    pub fn apply_to_colors(&self, colors: &mut Colors, tool: Tool) -> bool {
        let Some(picked) = self.last_picked else {
            return false;
        };
        colors.choose_color(tool, picked.to_hex());
        true
    }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::darkmode::{self, PageColors, Pictures};
    use ecclesia_ink::erase::Point;
    use ecclesia_ink::fill::{
        ColorStop, ExtendMode, Fill, FillRule, LinearGradient, RadialGradient, SolidFill,
        build_appearance,
    };

    use super::*;

    /// Builds a minimal single-page PDF containing one Form XObject painted onto the page, so MuPDF can render it.
    fn single_form_pdf(form_xobject: &str, page_w: f32, page_h: f32) -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offsets = Vec::new();
        let mut push_obj = |body: &str| {
            offsets.push(pdf.len());
            let id = offsets.len();
            pdf.push_str(&format!("{id} 0 obj\n{body}\nendobj\n"));
        };
        push_obj("<< /Type /Catalog /Pages 2 0 R >>");
        push_obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        push_obj(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w} {page_h}] /Resources << /XObject << /Fm0 5 0 R >> >> /Contents 4 0 R >>"
        ));
        let page_content = "q /Fm0 Do Q\n";
        push_obj(&format!(
            "<< /Length {} >>\nstream\n{page_content}endstream",
            page_content.len()
        ));
        push_obj(form_xobject);

        let xref_pos = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n", offsets.len() + 1));
        pdf.push_str("0000000000 65535 f \n");
        for off in offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n"
        ));
        pdf.into_bytes()
    }

    #[test]
    fn picked_color_is_exact_from_pixmap_before_dark_mode() {
        // Exact target color #2a7b4f painted on a 200x200 PDF page.
        let target = SampledColor::from_hex(0x2a7b4f);
        let rect_path = vec![
            Point { x: 20.0, y: 20.0 },
            Point { x: 180.0, y: 20.0 },
            Point { x: 180.0, y: 180.0 },
            Point { x: 20.0, y: 180.0 },
        ];
        let appearance = build_appearance(
            &rect_path,
            &Fill::Solid(SolidFill::new(target.to_rgb_f32(), 1.0)),
            FillRule::NonZero,
            None,
            200.0,
        )
        .expect("builds appearance");
        let pdf_bytes = single_form_pdf(&appearance.to_form_xobject(), 200.0, 200.0);

        let doc = ecclesia_mupdf::Document::from_bytes(&pdf_bytes, "swatch.pdf")
            .expect("MuPDF opens PDF");
        let page = doc.load_page(0).expect("loads page 0");
        let list = page.display_list(None).expect("display list");

        // Render a tile and apply smart dark mode to prove dark mode alters displayed pixels.
        let mut dark_tile = list
            .render_area(
                &Matrix::IDENTITY,
                DeviceRect {
                    x0: 0,
                    y0: 0,
                    x1: 200,
                    y1: 200,
                },
                Background::White,
                None,
            )
            .expect("renders tile");
        let pre_dark = sample_pixmap(&dark_tile, 100, 100, SampleArea::SinglePixel).expect("pre");
        assert_eq!(pre_dark, target, "pre-dark-mode pixmap matches exact RGB");

        let dark_colors = PageColors {
            ink: [230, 230, 235],
            paper: [24, 24, 28],
            link: None,
            pictures: Pictures::Recolor,
        };
        let (w, h, stride, comps) = (
            dark_tile.width() as usize,
            dark_tile.height() as usize,
            dark_tile.stride(),
            dark_tile.components(),
        );
        darkmode::recolor(
            dark_tile.samples_mut(),
            w,
            h,
            stride,
            comps,
            &dark_colors,
            None,
        );
        let post_dark = sample_pixmap(&dark_tile, 100, 100, SampleArea::SinglePixel).expect("post");
        assert_ne!(
            post_dark, target,
            "dark mode visibly altered the displayed tile"
        );

        // Eyedropper samples directly from the DisplayList pre-dark-mode and gets the EXACT original color.
        let picked = sample_page_point(
            &list,
            PagePoint { x: 100.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .expect("samples page point");
        assert_eq!(picked, target);
        assert_eq!(picked.to_hex_string(), "#2a7b4f");
    }

    #[test]
    fn h16_linear_and_radial_gradient_appearance_streams_render_in_mupdf() {
        let rect_path = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 200.0, y: 0.0 },
            Point { x: 200.0, y: 200.0 },
            Point { x: 0.0, y: 200.0 },
        ];
        // Linear red -> blue across x = 0..200.
        let linear = build_appearance(
            &rect_path,
            &Fill::Linear(LinearGradient {
                start: Point { x: 0.0, y: 100.0 },
                end: Point { x: 200.0, y: 100.0 },
                stops: vec![
                    ColorStop::new(0.0, [1.0, 0.0, 0.0]),
                    ColorStop::new(1.0, [0.0, 0.0, 1.0]),
                ],
                extend: ExtendMode::Both,
                opacity: 1.0,
            }),
            FillRule::NonZero,
            None,
            200.0,
        )
        .expect("builds linear appearance");
        let doc = ecclesia_mupdf::Document::from_bytes(
            &single_form_pdf(&linear.to_form_xobject(), 200.0, 200.0),
            "linear.pdf",
        )
        .expect("opens linear PDF");
        let list = doc.load_page(0).unwrap().display_list(None).unwrap();
        let left = sample_page_point(
            &list,
            PagePoint { x: 5.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .unwrap();
        let mid = sample_page_point(
            &list,
            PagePoint { x: 100.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .unwrap();
        let right = sample_page_point(
            &list,
            PagePoint { x: 195.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .unwrap();
        assert!(left.r > 230 && left.b < 25, "left is red: {left:?}");
        assert!(
            (i32::from(mid.r) - i32::from(mid.b)).abs() < 20,
            "mid blends red+blue: {mid:?}"
        );
        assert!(right.b > 230 && right.r < 25, "right is blue: {right:?}");

        // Radial white center -> green edge.
        let radial = build_appearance(
            &rect_path,
            &Fill::Radial(RadialGradient {
                start_center: Point { x: 100.0, y: 100.0 },
                start_radius: 0.0,
                end_center: Point { x: 100.0, y: 100.0 },
                end_radius: 90.0,
                stops: vec![
                    ColorStop::new(0.0, [1.0, 1.0, 1.0]),
                    ColorStop::new(1.0, [0.0, 0.8, 0.0]),
                ],
                extend: ExtendMode::Both,
                opacity: 1.0,
            }),
            FillRule::NonZero,
            None,
            200.0,
        )
        .expect("builds radial appearance");
        let doc = ecclesia_mupdf::Document::from_bytes(
            &single_form_pdf(&radial.to_form_xobject(), 200.0, 200.0),
            "radial.pdf",
        )
        .expect("opens radial PDF");
        let list = doc.load_page(0).unwrap().display_list(None).unwrap();
        let center = sample_page_point(
            &list,
            PagePoint { x: 100.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .unwrap();
        let rim = sample_page_point(
            &list,
            PagePoint { x: 190.0, y: 100.0 },
            1.0,
            SampleArea::SinglePixel,
        )
        .unwrap();
        assert!(
            center.r > 240 && center.g > 240 && center.b > 240,
            "{center:?}"
        );
        assert!(rim.g > 180 && rim.r < 25 && rim.b < 25, "{rim:?}");
    }

    #[test]
    fn buffer_sampling_loupe_and_tool_integration_work() {
        // 3x3 RGB buffer with stride padding (12 bytes per row instead of 9).
        let mut raw = vec![0u8; 3 * 12];
        for y in 0..3 {
            for x in 0..3 {
                let off = y * 12 + x * 3;
                raw[off] = (x * 60) as u8;
                raw[off + 1] = (y * 80) as u8;
                raw[off + 2] = 100;
            }
        }
        let buf = PixelBuffer {
            samples: &raw,
            width: 3,
            height: 3,
            stride: 12,
            format: PixelFormat::Rgb,
        };
        assert_eq!(
            sample_pixel(&buf, 1, 1, SampleArea::SinglePixel),
            Some(SampledColor::from_rgb(60, 80, 100))
        );
        assert_eq!(
            sample_pixel(&buf, 1, 1, SampleArea::Average3x3),
            Some(SampledColor::from_rgb(60, 80, 100))
        );
        assert_eq!(sample_pixel(&buf, 5, 5, SampleArea::SinglePixel), None);

        let grid = loupe_grid(&buf, 0, 0, 1);
        assert_eq!(grid.side, 3);
        assert_eq!(grid.pixels.len(), 9);
        assert_eq!(grid.pixels[0], None, "top-left out of bounds");
        assert_eq!(grid.center, Some(SampledColor::from_rgb(0, 0, 100)));

        let mut state = EyedropperState::new();
        state.activate();
        assert!(state.is_active());
        let picked = state.pick_at(&buf, 2, 1).expect("picks");
        assert_eq!(picked, SampledColor::from_rgb(120, 80, 100));
        assert!(!state.is_active());

        let mut favs = Favorites::default();
        assert!(state.apply_to_favorites(&mut favs));
        assert_eq!(favs.in_hand().color, picked.to_hex());

        let mut colors = Colors::default();
        assert!(state.apply_to_colors(&mut colors, Tool::Shape));
        assert_eq!(colors.style(Tool::Shape).color, picked.to_hex());
    }
}
