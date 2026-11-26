//! Infinite canvas pages and whiteboards (plan N.5).
//!
//! Coordinates are stored in `f64` world points so strokes drawn millions of points from the
//! origin retain full sub-point precision. On export to PDF, coordinates are translated relative to
//! the content bounding box in `f64` before emitting vector paths, supporting either a single
//! large page or multi-page paper-sized tiling.

use std::fmt::Write as _;

use excellent_core::notebook::{
    NotebookError, Orientation, PageSize, PdfBuilder, PdfPageInput, Result,
};

/// A single high-precision point on an infinite canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPoint {
    /// World X in points (`f64` for full precision far from origin).
    pub x: f64,
    /// World Y in points (y grows down, matching page space).
    pub y: f64,
    /// Stroke width at this point in points.
    pub width: f32,
}

/// An ink stroke stored in infinite canvas world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasStroke {
    /// Stable stroke identifier.
    pub id: String,
    /// Stroke RGB color in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Stroke opacity in `0.0..=1.0`.
    pub opacity: f32,
    /// Points in drawing order.
    pub points: Vec<CanvasPoint>,
}

impl CanvasStroke {
    /// Axis-aligned bounding box of this stroke `(min_x, min_y, max_x, max_y)` including stroke half-widths.
    #[must_use]
    pub fn bounds(&self) -> Option<WorldRect> {
        let first = self.points.first()?;
        let hw0 = f64::from(first.width) * 0.5;
        let mut rect = WorldRect {
            min_x: first.x - hw0,
            min_y: first.y - hw0,
            max_x: first.x + hw0,
            max_y: first.y + hw0,
        };
        for pt in &self.points[1..] {
            let hw = f64::from(pt.width) * 0.5;
            rect.min_x = rect.min_x.min(pt.x - hw);
            rect.min_y = rect.min_y.min(pt.y - hw);
            rect.max_x = rect.max_x.max(pt.x + hw);
            rect.max_y = rect.max_y.max(pt.y + hw);
        }
        Some(rect)
    }
}

/// Axis-aligned bounding rectangle in `f64` world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldRect {
    /// Minimum X coordinate.
    pub min_x: f64,
    /// Minimum Y coordinate.
    pub min_y: f64,
    /// Maximum X coordinate.
    pub max_x: f64,
    /// Maximum Y coordinate.
    pub max_y: f64,
}

impl WorldRect {
    /// Width in world points.
    #[must_use]
    pub fn width(self) -> f64 {
        (self.max_x - self.min_x).max(0.0)
    }

    /// Height in world points.
    #[must_use]
    pub fn height(self) -> f64 {
        (self.max_y - self.min_y).max(0.0)
    }

    /// Returns the union of `self` and `other`.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Expands all four sides by `padding`.
    #[must_use]
    pub fn padded(self, padding: f64) -> Self {
        let p = padding.max(0.0);
        Self {
            min_x: self.min_x - p,
            min_y: self.min_y - p,
            max_x: self.max_x + p,
            max_y: self.max_y + p,
        }
    }

    /// Whether `self` intersects `other`.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }
}

/// Pan and zoom camera over an [`InfiniteCanvas`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasViewport {
    /// World X at the center of the viewport.
    pub center_x: f64,
    /// World Y at the center of the viewport.
    pub center_y: f64,
    /// Zoom factor (`1.0` = 100%).
    pub zoom: f64,
    /// Viewport width in screen points.
    pub view_width: f64,
    /// Viewport height in screen points.
    pub view_height: f64,
}

impl Default for CanvasViewport {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
            view_width: 1280.0,
            view_height: 800.0,
        }
    }
}

impl CanvasViewport {
    /// Pans the viewport by `(dx_screen, dy_screen)` screen points.
    pub fn pan(&mut self, dx_screen: f64, dy_screen: f64) {
        let z = self.zoom.max(0.01);
        self.center_x -= dx_screen / z;
        self.center_y -= dy_screen / z;
    }

    /// Zooms smoothly around a screen anchor `(anchor_x, anchor_y)` clamped to `[0.05, 32.0]`.
    pub fn zoom_at(&mut self, factor: f64, anchor_x: f64, anchor_y: f64) {
        let world_before = self.screen_to_world(anchor_x, anchor_y);
        self.zoom = (self.zoom * factor).clamp(0.05, 32.0);
        let world_after = self.screen_to_world(anchor_x, anchor_y);
        self.center_x += world_before.0 - world_after.0;
        self.center_y += world_before.1 - world_after.1;
    }

    /// Converts screen coordinates to high-precision world coordinates.
    #[must_use]
    pub fn screen_to_world(self, sx: f64, sy: f64) -> (f64, f64) {
        let z = self.zoom.max(0.01);
        (
            self.center_x + (sx - self.view_width * 0.5) / z,
            self.center_y + (sy - self.view_height * 0.5) / z,
        )
    }

    /// Converts world coordinates to screen coordinates.
    #[must_use]
    pub fn world_to_screen(self, wx: f64, wy: f64) -> (f64, f64) {
        (
            (wx - self.center_x) * self.zoom + self.view_width * 0.5,
            (wy - self.center_y) * self.zoom + self.view_height * 0.5,
        )
    }

    /// Returns the currently visible world rectangle.
    #[must_use]
    pub fn visible_world_rect(self) -> WorldRect {
        let (x0, y0) = self.screen_to_world(0.0, 0.0);
        let (x1, y1) = self.screen_to_world(self.view_width, self.view_height);
        WorldRect {
            min_x: x0.min(x1),
            min_y: y0.min(y1),
            max_x: x0.max(x1),
            max_y: y0.max(y1),
        }
    }
}

/// Normalized minimap projection for R.17 orientation over an infinite canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapProjection {
    /// Combined world bounds shown in the minimap.
    pub world_bounds: WorldRect,
    /// Viewport rectangle normalized to `[0.0..=1.0]` within `world_bounds`.
    pub viewport_norm: [f64; 4],
}

/// How an [`InfiniteCanvas`] is exported to PDF (plan N.5 step 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InfiniteExportMode {
    /// Export the entire content bounding box onto one large PDF page.
    SinglePage {
        /// Margin padding around content bounds in points.
        padding_pt: f64,
    },
    /// Split the content bounding box into a grid of pages of the chosen paper size.
    Tiled {
        /// Paper size per tile page.
        paper: PageSize,
        /// Paper orientation per tile page.
        orientation: Orientation,
        /// Page margin on each tile in points.
        margin_pt: f64,
    },
}

/// An unbounded infinite canvas / whiteboard page (plan N.5).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InfiniteCanvas {
    /// All strokes on the canvas.
    pub strokes: Vec<CanvasStroke>,
    /// Current viewport camera.
    pub viewport: CanvasViewport,
}

impl InfiniteCanvas {
    /// Creates a new empty infinite canvas.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a stroke to the canvas.
    pub fn push_stroke(&mut self, stroke: CanvasStroke) {
        self.strokes.push(stroke);
    }

    /// Computes the exact bounding box of all strokes on the canvas (`None` if empty).
    #[must_use]
    pub fn content_bounds(&self) -> Option<WorldRect> {
        let mut iter = self.strokes.iter().filter_map(CanvasStroke::bounds);
        let first = iter.next()?;
        Some(iter.fold(first, WorldRect::union))
    }

    /// Computes the minimap projection covering both content and the current viewport.
    #[must_use]
    pub fn minimap(&self) -> MinimapProjection {
        let vis = self.viewport.visible_world_rect();
        let combined = match self.content_bounds() {
            Some(content) => content.union(vis),
            None => vis,
        };
        let w = combined.width().max(1.0);
        let h = combined.height().max(1.0);
        MinimapProjection {
            world_bounds: combined,
            viewport_norm: [
                (vis.min_x - combined.min_x) / w,
                (vis.min_y - combined.min_y) / h,
                (vis.max_x - combined.min_x) / w,
                (vis.max_y - combined.min_y) / h,
            ],
        }
    }

    /// Exports all strokes on this infinite canvas to a valid vector PDF, preserving every stroke
    /// and full relative precision even when drawn far from the origin.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidFormat`] if PDF construction fails.
    pub fn export_pdf(&self, mode: InfiniteExportMode) -> Result<Vec<u8>> {
        let content_rect = self.content_bounds().unwrap_or(WorldRect {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 612.0,
            max_y: 792.0,
        });

        let mut builder = PdfBuilder::new();
        match mode {
            InfiniteExportMode::SinglePage { padding_pt } => {
                let bounds = content_rect.padded(padding_pt);
                let page_w = bounds.width().max(72.0);
                let page_h = bounds.height().max(72.0);
                let stream = emit_strokes_in_tile(&self.strokes, bounds, page_h, 0.0, 0.0);
                builder.add_page(PdfPageInput {
                    width: page_w as f32,
                    height: page_h as f32,
                    content: stream,
                    crop_box: None,
                });
            }
            InfiniteExportMode::Tiled {
                paper,
                orientation,
                margin_pt,
            } => {
                let (pw, ph) = paper.dimensions(orientation);
                let pw_f64 = f64::from(pw);
                let ph_f64 = f64::from(ph);
                let m = margin_pt.clamp(0.0, (pw_f64.min(ph_f64) * 0.4).max(0.0));
                let tile_w = (pw_f64 - 2.0 * m).max(36.0);
                let tile_h = (ph_f64 - 2.0 * m).max(36.0);

                let cols = ((content_rect.width() / tile_w).ceil() as usize).max(1);
                let rows = ((content_rect.height() / tile_h).ceil() as usize).max(1);

                for row in 0..rows {
                    for col in 0..cols {
                        let tile = WorldRect {
                            min_x: content_rect.min_x + col as f64 * tile_w,
                            min_y: content_rect.min_y + row as f64 * tile_h,
                            max_x: content_rect.min_x + (col + 1) as f64 * tile_w,
                            max_y: content_rect.min_y + (row + 1) as f64 * tile_h,
                        };
                        let mut stream = String::new();
                        // Clip to tile printable area.
                        let _ = writeln!(
                            stream,
                            "q\n{:.3} {:.3} {:.3} {:.3} re W n",
                            m, m, tile_w, tile_h
                        );
                        stream.push_str(&emit_strokes_in_tile(&self.strokes, tile, ph_f64, m, m));
                        stream.push_str("Q\n");
                        builder.add_page(PdfPageInput {
                            width: pw,
                            height: ph,
                            content: stream,
                            crop_box: None,
                        });
                    }
                }
            }
        }

        builder.build().map_err(|e| {
            NotebookError::InvalidFormat(format!("infinite canvas PDF export failed: {e}"))
        })
    }
}

fn emit_strokes_in_tile(
    strokes: &[CanvasStroke],
    tile_world: WorldRect,
    page_height: f64,
    offset_x: f64,
    offset_y_top: f64,
) -> String {
    let mut out = String::new();
    for stroke in strokes {
        let Some(sb) = stroke.bounds() else {
            continue;
        };
        if !sb.intersects(tile_world) {
            continue;
        }
        let [r, g, b] = stroke.color;
        let mean_w = if stroke.points.is_empty() {
            1.5
        } else {
            stroke.points.iter().map(|p| p.width).sum::<f32>() / stroke.points.len() as f32
        };
        let _ = writeln!(
            out,
            "{r:.4} {g:.4} {b:.4} RG {:.3} w 1 J 1 j",
            mean_w.max(0.25)
        );
        for (i, pt) in stroke.points.iter().enumerate() {
            // Translate in f64 BEFORE converting to PDF coordinates to preserve full precision
            // far from the world origin!
            let px = (pt.x - tile_world.min_x) + offset_x;
            let py = page_height - ((pt.y - tile_world.min_y) + offset_y_top);
            if i == 0 {
                let _ = writeln!(out, "{px:.3} {py:.3} m");
            } else {
                let _ = writeln!(out, "{px:.3} {py:.3} l");
            }
        }
        if stroke.points.len() == 1 {
            let pt = stroke.points[0];
            let px = (pt.x - tile_world.min_x) + offset_x;
            let py = page_height - ((pt.y - tile_world.min_y) + offset_y_top);
            let _ = writeln!(out, "{:.3} {:.3} l", px + 0.1, py);
        }
        out.push_str("S\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use excellent_mupdf::{Background, Document, Matrix};

    #[test]
    fn drawing_far_from_origin_keeps_full_precision_and_exports_every_stroke() {
        let mut canvas = InfiniteCanvas::new();
        let origin_x = 10_000_000.0_f64;
        let origin_y = -5_000_000.0_f64;

        // Draw two distinct strokes separated by 0.25 pt far from origin (would suffer f32 loss!).
        canvas.push_stroke(CanvasStroke {
            id: "s1".into(),
            color: [0.1, 0.2, 0.8],
            opacity: 1.0,
            points: vec![
                CanvasPoint {
                    x: origin_x,
                    y: origin_y,
                    width: 3.0,
                },
                CanvasPoint {
                    x: origin_x + 120.25,
                    y: origin_y + 80.75,
                    width: 3.0,
                },
            ],
        });
        canvas.push_stroke(CanvasStroke {
            id: "s2".into(),
            color: [0.8, 0.1, 0.1],
            opacity: 1.0,
            points: vec![
                CanvasPoint {
                    x: origin_x + 900.0,
                    y: origin_y + 600.0,
                    width: 4.0,
                },
                CanvasPoint {
                    x: origin_x + 1100.5,
                    y: origin_y + 750.5,
                    width: 4.0,
                },
            ],
        });

        // Verify f64 precision far from origin.
        let bounds = canvas.content_bounds().expect("bounds");
        assert!((bounds.width() - (1100.5 + 3.5)).abs() < 0.01, "{bounds:?}");
        assert!((bounds.height() - (750.5 + 3.5)).abs() < 0.01, "{bounds:?}");

        // Export as single large page and verify MuPDF renders non-blank ink pixels.
        let single_pdf = canvas
            .export_pdf(InfiniteExportMode::SinglePage { padding_pt: 24.0 })
            .expect("single page pdf");
        let doc = Document::from_bytes(&single_pdf, "application/pdf").expect("open single");
        assert_eq!(doc.page_count().expect("pages"), 1);
        let pix = doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::scale(0.5, 0.5), Background::White)
            .expect("render");
        let inked = pix
            .samples()
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|px| **px != [255, 255, 255])
            .count();
        assert!(inked > 50, "single page export must render both strokes");

        // Export tiled onto Letter pages (1104 x 754 pt with 36 pt margins -> 3x2 = 6 tiles).
        let tiled_pdf = canvas
            .export_pdf(InfiniteExportMode::Tiled {
                paper: PageSize::Letter,
                orientation: Orientation::Portrait,
                margin_pt: 36.0,
            })
            .expect("tiled pdf");
        let tiled_doc = Document::from_bytes(&tiled_pdf, "application/pdf").expect("open tiled");
        assert!(tiled_doc.page_count().expect("tile pages") >= 4);
    }

    #[test]
    fn viewport_pan_zoom_and_minimap_stay_consistent() {
        let mut canvas = InfiniteCanvas::new();
        canvas.viewport.zoom_at(2.0, 640.0, 400.0);
        assert!((canvas.viewport.zoom - 2.0).abs() < 1e-6);
        let (wx, wy) = canvas.viewport.screen_to_world(320.0, 200.0);
        let (sx, sy) = canvas.viewport.world_to_screen(wx, wy);
        assert!((sx - 320.0).abs() < 1e-6 && (sy - 200.0).abs() < 1e-6);
        let mm = canvas.minimap();
        assert!(mm.viewport_norm[0] >= 0.0 && mm.viewport_norm[2] <= 1.0 + 1e-6);
    }
}
