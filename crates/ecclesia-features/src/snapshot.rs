//! Region snapshots (O.23, parity with SumatraPDF Enhanced's screenshots): a region dragged on a page is rendered
//! from the page's display list at a chosen resolution, not grabbed from the screen, so a vector figure stays sharp at
//! any size. The picture is copied as an image or saved as PNG.

use ecclesia_mupdf::{Background, Document, Matrix, PageRect, Pixmap, Result};
use serde_json::Value;

/// The resolutions offered, in dots per inch: the screen's, then print-quality ones.
pub const RESOLUTIONS: [u32; 4] = [96, 150, 300, 600];

/// The resolution offered first.
pub const DEFAULT_DPI: u32 = 300;

/// The most pixels a snapshot may have (about 40 megapixels, 160 MB in RGBA): a larger region at a high resolution is
/// rendered at the highest resolution that fits.
pub const MAX_PIXELS: f64 = 40.0e6;

/// Points per inch: a PDF page's unit.
const POINTS_PER_INCH: f32 = 72.0;

/// A region the reader dragged out on a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// The page, counting from zero.
    pub page: usize,
    /// The region, in page points, either way round.
    pub rect: PageRect,
}

impl Region {
    /// The region the canvas reports (`snapshot`: `{page, rect: [x0, y0, x1, y1]}`, page points); `None` when it
    /// doesn't parse or has no area.
    #[must_use]
    pub fn parse(data: &Value) -> Option<Self> {
        let numbers: Vec<f32> = data
            .get("rect")?
            .as_array()?
            .iter()
            .filter_map(|n| n.as_f64().map(|n| n as f32))
            .collect();
        let [x0, y0, x1, y1] = <[f32; 4]>::try_from(numbers).ok()?;
        let rect = PageRect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        };
        (rect.x1 - rect.x0 >= 1.0 && rect.y1 - rect.y0 >= 1.0).then_some(Self {
            page: usize::try_from(data.get("page")?.as_u64()?).ok()?,
            rect,
        })
    }

    /// The resolution it is rendered at for a request of `dpi`: `dpi`, or less when that would pass [`MAX_PIXELS`].
    #[must_use]
    pub fn fitted_dpi(&self, dpi: u32) -> f32 {
        let inches = (self.rect.x1 - self.rect.x0) * (self.rect.y1 - self.rect.y0)
            / (POINTS_PER_INCH * POINTS_PER_INCH);
        let most = (MAX_PIXELS / f64::from(inches.max(f32::EPSILON))).sqrt() as f32;
        (dpi as f32).min(most)
    }

    /// Its size in pixels at `dpi` (after [`Region::fitted_dpi`]).
    #[must_use]
    pub fn pixels(&self, dpi: u32) -> (u32, u32) {
        let scale = self.fitted_dpi(dpi) / POINTS_PER_INCH;
        (
            ((self.rect.x1 - self.rect.x0) * scale).round() as u32,
            ((self.rect.y1 - self.rect.y0) * scale).round() as u32,
        )
    }

    /// The region of `document` rendered at `dpi` on white, as PNG. Slow for a large region at a high resolution:
    /// run it on a worker.
    ///
    /// # Errors
    ///
    /// MuPDF can't load or render the page, or encode the picture.
    pub fn render_png(&self, document: &Document, dpi: u32) -> Result<Vec<u8>> {
        self.render(document, dpi)?.to_png()
    }

    /// The region of `document` rendered at `dpi` on white.
    ///
    /// # Errors
    ///
    /// MuPDF can't load or render the page.
    pub fn render(&self, document: &Document, dpi: u32) -> Result<Pixmap> {
        let scale = self.fitted_dpi(dpi) / POINTS_PER_INCH;
        let ctm = Matrix::scale(scale, scale);
        let list = document.load_page(self.page)?.display_list(None)?;
        let area = ctm.apply_rect(&self.rect);
        list.render_area(&ctm, area, Background::White, None)
    }

    /// A file name for it: the document's name, its page and "snapshot".
    #[must_use]
    pub fn file_name(&self, document_stem: &str) -> String {
        format!("{document_stem} p{} snapshot.png", self.page + 1)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_region_reads_either_way_round_and_needs_an_area() {
        let region =
            Region::parse(&json!({"page": 1, "rect": [200, 300, 100, 100]})).expect("parses");
        assert_eq!(
            region.rect,
            PageRect {
                x0: 100.0,
                y0: 100.0,
                x1: 200.0,
                y1: 300.0
            }
        );
        assert_eq!(
            Region::parse(&json!({"page": 0, "rect": [5, 5, 5, 50]})),
            None
        );
        assert_eq!(region.file_name("Notes"), "Notes p2 snapshot.png");
    }

    #[test]
    fn a_huge_region_is_rendered_as_large_as_fits() {
        let page = Region {
            page: 0,
            rect: PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 2000.0,
                y1: 2000.0,
            },
        };
        let (w, h) = page.pixels(600);
        assert!(
            f64::from(w) * f64::from(h) <= MAX_PIXELS * 1.01,
            "{w} x {h}"
        );
        let small = Region {
            page: 0,
            rect: PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 72.0,
                y1: 36.0,
            },
        };
        assert_eq!(
            small.pixels(600),
            (600, 300),
            "an inch by half an inch at 600 DPI"
        );
    }
}
