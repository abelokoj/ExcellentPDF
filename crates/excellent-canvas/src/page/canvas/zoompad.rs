//! The zoom writing pad on the canvas (H.12): while it is on (the `zoomPad` prop) the bottom third of the view is a
//! pad. A pen stroke begun there is written large and lands small in the box outlined on the page; the pad shows what
//! has been written in the box, magnified, and the box steps along the line by itself. The model is `excellent-ink`'s
//! `zoompad`.

use excellent_ink::erase::Point;
use excellent_ink::stroke::InkPoint;
use excellent_ink::zoompad::{MAGNIFY, ZoomPad};
use excellent_mupdf::PagePoint;

use super::{InkPixel, PageCanvas, PenSample};
use crate::Rect;
use crate::page::host::Host;
use crate::page::placement::Placement;

/// The pad's share of the view's height.
const PAD_SHARE: f32 = 0.32;
/// The space round the pad, in pixels.
const PAD_MARGIN: f32 = 12.0;
/// The pad's background and edge.
const PAD_PAPER: u32 = 0xfbfaf6;
const PAD_EDGE: u32 = 0x9aa9bb;
/// The page's writing area, as shares of the page: margins left, right and at the foot.
const MARGINS: (f32, f32, f32) = (0.1, 0.9, 0.92);

/// The pad: its box on a page, and the strokes written in it (page points, with their colours) to show magnified.
#[derive(Debug)]
pub(super) struct Pad {
    zoom: ZoomPad,
    page: usize,
    ink: Vec<(Vec<InkPoint>, u32)>,
}

/// What a frame draws for the pad: its area, the box's outline on the page, and the strokes, in screen pixels.
pub(super) struct PadShapes {
    area: Rect,
    outline: Vec<InkPixel>,
    strokes: Vec<(Vec<InkPixel>, u32)>,
}

impl<H: Host> PageCanvas<H> {
    /// Turns the pad on (on the page in view, its box at the top of the writing area) or off (the `zoomPad` prop).
    pub(crate) fn set_zoom_pad(&mut self, on: bool) {
        if on == self.pad.is_some() {
            return;
        }
        self.pad = if on { self.new_pad() } else { None };
        self.invalidate();
    }

    fn new_pad(&self) -> Option<Pad> {
        let page = self.reported_page.unwrap_or(0);
        let bounds = self.bounds.get(page)?;
        let (w, h) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        let area = self.pad_area();
        let margins = (
            bounds.x0 + w * MARGINS.0,
            bounds.x0 + w * MARGINS.1,
            bounds.y0 + h * MARGINS.2,
        );
        let corner = Point {
            x: margins.0,
            y: bounds.y0 + h * 0.1,
        };
        Some(Pad {
            zoom: ZoomPad::new(
                (area.width(), area.height()),
                self.viewport.scale(),
                corner,
                margins,
            ),
            page,
            ink: Vec::new(),
        })
    }

    /// The pad's area on screen: the bottom third of the view, inset.
    fn pad_area(&self) -> Rect {
        let (w, h) = (self.viewport.width(), self.viewport.height());
        Rect {
            x0: PAD_MARGIN,
            y0: h * (1.0 - PAD_SHARE),
            x1: w - PAD_MARGIN,
            y1: h - PAD_MARGIN,
        }
    }

    /// The page a pen touching down at `sample` writes on through the pad, when the pad is on and it touched it.
    pub(super) fn pad_page(&self, sample: &PenSample) -> Option<usize> {
        let pad = self.pad.as_ref()?;
        let area = self.pad_area();
        let inside =
            (area.x0..=area.x1).contains(&sample.x) && (area.y0..=area.y1).contains(&sample.y);
        inside.then_some(pad.page)
    }

    /// Where pad pixel (`x`, `y`) of the view lands on the page.
    pub(super) fn pad_point(&self, x: f32, y: f32) -> Option<PagePoint> {
        let pad = self.pad.as_ref()?;
        let area = self.pad_area();
        let p = pad.zoom.to_page(
            Point {
                x: x - area.x0,
                y: y - area.y0,
            },
            (area.width(), area.height()),
        );
        Some(PagePoint { x: p.x, y: p.y })
    }

    /// A stroke written in the pad is finished: its widths come down to the page's scale, it is kept to show, and
    /// the box steps on when the writing neared the pad's right edge. Returns the points to save.
    pub(super) fn pad_done(&mut self, points: Vec<InkPoint>, color: u32) -> Vec<InkPoint> {
        let area = self.pad_area();
        let Some(pad) = &mut self.pad else {
            return points;
        };
        let points: Vec<InkPoint> = points
            .into_iter()
            .map(|p| InkPoint {
                width: p.width / MAGNIFY,
                ..p
            })
            .collect();
        let (left, _, right, _) = pad.zoom.target();
        let right_most = points.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let across = (right_most - left) / (right - left).max(f32::EPSILON) * area.width();
        pad.ink.push((points.clone(), color));
        if pad.zoom.stroke_done(across, area.width()) {
            // Keep what still shows in the box; the rest is on the page.
            let (x0, y0, x1, y1) = pad.zoom.target();
            pad.ink.retain(|(stroke, _)| {
                stroke
                    .iter()
                    .any(|p| p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1)
            });
        }
        self.invalidate();
        points
    }

    /// The pad as this frame draws it; `None` while it is off.
    pub(super) fn pad_shapes(&self) -> Option<PadShapes> {
        let pad = self.pad.as_ref()?;
        let area = self.pad_area();
        let (x0, y0, x1, y1) = pad.zoom.target();
        let (bw, bh) = (x1 - x0, y1 - y0);
        let placement =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, pad.page)?;
        let on_screen = |x: f32, y: f32| {
            let (lx, ly) = placement.to_layout(PagePoint { x, y });
            let (sx, sy) = self.viewport.to_screen_point(lx, ly);
            (sx, sy, 1.5)
        };
        let outline = vec![
            on_screen(x0, y0),
            on_screen(x1, y0),
            on_screen(x1, y1),
            on_screen(x0, y1),
            on_screen(x0, y0),
        ];
        let in_pad = |p: &InkPoint| {
            (
                area.x0 + (p.x - x0) / bw * area.width(),
                area.y0 + (p.y - y0) / bh * area.height(),
                p.width * MAGNIFY * self.viewport.scale(),
            )
        };
        let strokes = pad
            .ink
            .iter()
            .map(|(stroke, color)| (stroke.iter().map(in_pad).collect(), *color))
            .collect();
        Some(PadShapes {
            area,
            outline,
            strokes,
        })
    }

    /// Draws the pad's shapes on `host`: the box's outline on the page, then the pad and what is written in it.
    pub(super) fn draw_pad(host: &mut H, shapes: Option<PadShapes>) -> Result<(), H::Error> {
        let Some(shapes) = shapes else {
            return Ok(());
        };
        host.polyline(&shapes.outline, PAD_EDGE)?;
        host.fill(&shapes.area, PAD_PAPER)?;
        let a = shapes.area;
        host.polyline(
            &[
                (a.x0, a.y0, 1.5),
                (a.x1, a.y0, 1.5),
                (a.x1, a.y1, 1.5),
                (a.x0, a.y1, 1.5),
                (a.x0, a.y0, 1.5),
            ],
            PAD_EDGE,
        )?;
        // A writing line two thirds down, as on lined paper.
        let base = a.y0 + a.height() * 0.7;
        host.polyline(&[(a.x0, base, 1.0), (a.x1, base, 1.0)], 0xd7dde5)?;
        for (stroke, color) in &shapes.strokes {
            host.polyline(stroke, *color)?;
        }
        Ok(())
    }
}
