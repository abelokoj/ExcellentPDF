//! Drawing shapes on the pages (A.8, A.9): while the app has a drawing tool on (the `drawing` prop), a drag on a page
//! draws a line, an arrow, a rectangle or an ellipse there, shown as it goes in the shapes' colour (A.3). Shift snaps
//! a line to steps of 15° and makes a box a square or a circle. When the button comes up the canvas sends it to the
//! app (`pageDraw`: `{page, kind, points: [[x, y], [x, y]], color: [r, g, b], opacity}`, page points, where the drag
//! started and ended), which makes it an annotation. The model is `excellent-features`' `annotate::shapes`.

use excellent_features::annotate::colors::Tool as ColorTool;
use excellent_features::annotate::shapes::{self, Kind};
use excellent_mupdf::{LineEnd, PagePoint};

use super::PageCanvas;
use crate::page::host::{Host, InkPixel};
use crate::page::placement::Placement;

/// The drawing tool on, and the shape being dragged out.
#[derive(Debug, Default)]
pub(super) struct Drawing {
    tool: Option<Kind>,
    drag: Option<Drag>,
}

/// A shape being dragged out, in its page's points.
#[derive(Debug, Clone, Copy)]
struct Drag {
    page: usize,
    from: PagePoint,
    to: PagePoint,
}

/// An arrowhead's sides in the preview, as a multiple of the line's width, and their angle to the line.
const HEAD_LENGTH: f32 = 4.5;
const HEAD_ANGLE: f32 = 0.5;

impl<H: Host> PageCanvas<H> {
    /// The drawing tool the app has on (the `drawing` prop); `None` for none. A shape being dragged is dropped.
    pub(crate) fn set_drawing(&mut self, tool: Option<Kind>) {
        if self.drawing.tool != tool {
            self.drawing = Drawing { tool, drag: None };
            self.invalidate();
        }
    }

    /// Whether a drag on the pages draws: a tool is on and the document can take annotations.
    pub(super) fn is_drawing_tool(&self) -> bool {
        self.drawing.tool.is_some() && self.can_mark()
    }

    /// The left button went down at client pixel (`x`, `y`): with a drawing tool on, over a page, a shape starts
    /// there. Returns whether it did, when the window should capture the mouse until the button comes up.
    pub(super) fn draw_down(&mut self, x: i32, y: i32) -> bool {
        if !self.is_drawing_tool() {
            return false;
        }
        let Some((page, from)) = self.page_point(x, y) else {
            return false;
        };
        self.drawing.drag = Some(Drag {
            page,
            from,
            to: from,
        });
        true
    }

    /// The mouse moved to client pixel (`x`, `y`) (`None`: it left): a shape being dragged follows it. Returns
    /// whether one is, when the move does nothing else.
    pub(super) fn draw_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        let Some(drag) = self.drawing.drag else {
            return false;
        };
        if let Some(to) = at.and_then(|(x, y)| self.drag_end(drag, x, y))
            && to != drag.to
        {
            self.drawing.drag = Some(Drag { to, ..drag });
            self.invalidate();
        }
        true
    }

    /// The left button came up at client pixel (`x`, `y`): a shape being dragged ends there and goes to the app,
    /// unless it is too short to be one. Returns whether one was being dragged, when the window lets the mouse go.
    pub(super) fn draw_up(&mut self, x: i32, y: i32) -> bool {
        let Some(drag) = self.drawing.drag.take() else {
            return false;
        };
        self.invalidate();
        let to = self.drag_end(drag, x, y).unwrap_or(drag.to);
        let Some(kind) = self.drawing.tool else {
            return true;
        };
        if shapes::big_enough(kind, drag.from, to) {
            let style = self.markbar_style(ColorTool::Shape);
            self.emit(
                "pageDraw",
                serde_json::json!({
                    "page": drag.page,
                    "kind": kind.name(),
                    "points": [[drag.from.x, drag.from.y], [to.x, to.y]],
                    "color": style.rgb(),
                    "opacity": style.opacity,
                }),
            );
        }
        true
    }

    /// Something else took the mouse: the shape being dragged is dropped.
    pub(super) fn draw_cancel(&mut self) {
        if self.drawing.drag.take().is_some() {
            self.invalidate();
        }
    }

    /// Where the drag ends with the mouse at client pixel (`x`, `y`), in its page's points: while Shift is down, a
    /// line snapped and a box made square.
    fn drag_end(&self, drag: Drag, x: i32, y: i32) -> Option<PagePoint> {
        let (lx, ly) = self.viewport.to_layout_point(x as f32, y as f32);
        let placement =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, drag.page)?;
        let to = placement.to_page(lx, ly);
        Some(match self.drawing.tool {
            Some(kind) if self.host.shift_down() => shapes::constrain(kind, drag.from, to),
            _ => to,
        })
    }

    /// Draws the shape being dragged: its outline, and an arrow's head.
    pub(super) fn draw_shape_preview(&mut self) -> Result<(), H::Error> {
        let (Some(drag), Some(kind)) = (self.drawing.drag, self.drawing.tool) else {
            return Ok(());
        };
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, drag.page)
        else {
            return Ok(());
        };
        let screen = |point: PagePoint| {
            let (x, y) = placement.to_layout(point);
            self.viewport.to_screen_point(x, y)
        };
        let width = shapes::WIDTH * self.viewport.scale();
        let (from, to) = (screen(drag.from), screen(drag.to));
        let mut strokes: Vec<Vec<InkPixel>> = vec![
            shapes::outline(kind, drag.from, drag.to)
                .into_iter()
                .map(|point| {
                    let (x, y) = screen(point);
                    (x, y, width)
                })
                .collect(),
        ];
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx.hypot(dy);
        if kind.ends()[1] == LineEnd::OpenArrow && length > 0.0 {
            let back = dy.atan2(dx) + std::f32::consts::PI;
            let side = |turn: f32| {
                let angle = back + turn;
                let reach = (HEAD_LENGTH * width).min(length);
                (
                    to.0 + reach * angle.cos(),
                    to.1 + reach * angle.sin(),
                    width,
                )
            };
            strokes.push(vec![
                side(HEAD_ANGLE),
                (to.0, to.1, width),
                side(-HEAD_ANGLE),
            ]);
        }
        let color = self.markbar_style(ColorTool::Shape).color;
        for stroke in &strokes {
            self.host.polyline(stroke, color)?;
        }
        Ok(())
    }
}
