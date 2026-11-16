//! Drawing shapes on the pages (A.8, A.9, A.10): while the app has a drawing tool on (the `drawing` prop), a drag on
//! a page draws a line, an arrow, a rectangle, an ellipse, a regular polygon or a star there, and clicks put down a
//! polygon's, a polyline's or a cloud's corners one by one, each shown as it goes in the shapes' colour (A.3). Shift
//! snaps a line, or the side being clicked, to steps of 15°, and makes a box square. A double-click finishes a clicked
//! shape, as does a click on a polygon's first corner. Then the canvas sends it to the app (`pageDraw`: `{page, kind,
//! points: [[x, y], ...], color: [r, g, b], opacity}`, page points: where a drag started and ended, or each corner),
//! which makes it an annotation. The model is `excellent-features`' `annotate::shapes`.

use excellent_features::annotate::colors::Tool as ColorTool;
use excellent_features::annotate::shapes::{self, Kind};
use excellent_mupdf::{LineEnd, PagePoint};

use super::PageCanvas;
use crate::page::clicks::ClickLimits;
use crate::page::host::{Host, InkPixel};
use crate::page::placement::Placement;

/// The drawing tool on, and the shape being dragged out or clicked.
#[derive(Debug, Default)]
pub(super) struct Drawing {
    tool: Option<Kind>,
    drag: Option<Drag>,
    trace: Option<Trace>,
}

/// A shape being dragged out, in its page's points.
#[derive(Debug, Clone, Copy)]
struct Drag {
    page: usize,
    from: PagePoint,
    to: PagePoint,
}

/// A shape being clicked corner by corner, in its page's points, and where the next corner would go.
#[derive(Debug, Clone)]
struct Trace {
    page: usize,
    points: Vec<PagePoint>,
    next: Option<PagePoint>,
}

/// An arrowhead's sides in the preview, as a multiple of the line's width, and their angle to the line.
const HEAD_LENGTH: f32 = 4.5;
const HEAD_ANGLE: f32 = 0.5;

/// A click this close to a polygon's first corner, in pixels, closes it.
const CLOSE_PIXELS: f32 = 8.0;

impl<H: Host> PageCanvas<H> {
    /// The drawing tool the app has on (the `drawing` prop); `None` for none. A shape being drawn is dropped.
    pub(crate) fn set_drawing(&mut self, tool: Option<Kind>) {
        if self.drawing.tool != tool {
            self.drawing = Drawing {
                tool,
                ..Drawing::default()
            };
            self.invalidate();
        }
    }

    /// Whether the pages draw: a tool is on and the document can take annotations.
    pub(super) fn is_drawing_tool(&self) -> bool {
        self.drawing.tool.is_some() && self.can_mark()
    }

    /// The left button went down at client pixel (`x`, `y`), at message time `time`: with a drawing tool on, over a
    /// page, a dragged shape starts there, or a clicked shape gets a corner there (a double-click, or a click on a
    /// polygon's first corner, finishes it). Returns whether the press was the tool's, when the window should
    /// capture the mouse until the button comes up.
    pub(super) fn draw_down(&mut self, x: i32, y: i32, time: u32, limits: ClickLimits) -> bool {
        let Some(kind) = self.drawing.tool.filter(|_| self.is_drawing_tool()) else {
            return false;
        };
        if !kind.is_clicked() {
            let Some((page, from)) = self.page_point(x, y) else {
                return false;
            };
            self.drawing.drag = Some(Drag {
                page,
                from,
                to: from,
            });
            return true;
        }
        let clicks = self.clicks.press(time, x, y, limits);
        let Some(trace) = self.drawing.trace.take() else {
            if let Some((page, at)) = self.page_point(x, y) {
                self.drawing.trace = Some(Trace {
                    page,
                    points: vec![at],
                    next: None,
                });
                self.invalidate();
            }
            return true;
        };
        let at = self.trace_point(&trace, x, y);
        let closes = kind.is_closed()
            && trace.points.len() >= 3
            && self.screen_distance(trace.page, trace.points[0], x, y) <= CLOSE_PIXELS;
        if clicks >= 2 || closes {
            self.finish_trace(kind, trace);
        } else {
            let mut trace = trace;
            trace.points.extend(at);
            trace.next = None;
            self.drawing.trace = Some(trace);
        }
        self.invalidate();
        true
    }

    /// The mouse moved to client pixel (`x`, `y`) (`None`: it left): a shape being dragged follows it, and a clicked
    /// shape's next side reaches to it. Returns whether either is, when the move does nothing else.
    pub(super) fn draw_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        if let Some(trace) = &self.drawing.trace {
            let next = at.and_then(|(x, y)| self.trace_point(trace, x, y));
            if self.drawing.trace.as_ref().is_some_and(|t| t.next != next) {
                if let Some(trace) = &mut self.drawing.trace {
                    trace.next = next;
                }
                self.invalidate();
            }
            return true;
        }
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
    /// unless it is too small to be one. Returns whether the press was the tool's, when the window lets the mouse go.
    pub(super) fn draw_up(&mut self, x: i32, y: i32) -> bool {
        let Some(drag) = self.drawing.drag.take() else {
            // A clicked shape's corners go down with the button; its up is the tool's too.
            return self.drawing.trace.is_some()
                || self.drawing.tool.is_some_and(Kind::is_clicked) && self.is_drawing_tool();
        };
        self.invalidate();
        let to = self.drag_end(drag, x, y).unwrap_or(drag.to);
        let Some(kind) = self.drawing.tool else {
            return true;
        };
        if shapes::big_enough(kind, drag.from, to) {
            self.emit_drawn(kind, drag.page, &[drag.from, to]);
        }
        true
    }

    /// Something else took the mouse: the shape being dragged is dropped. A clicked shape's corners stay.
    pub(super) fn draw_cancel(&mut self) {
        if self.drawing.drag.take().is_some() {
            self.invalidate();
        }
    }

    /// The clicked shape `trace` is done: it goes to the app when its corners make one, and is dropped either way.
    fn finish_trace(&mut self, kind: Kind, trace: Trace) {
        if shapes::enough(kind, &trace.points) {
            self.emit_drawn(kind, trace.page, &trace.points);
        }
    }

    /// Sends the app the `kind` drawn through `points` on page `page` (`pageDraw`).
    fn emit_drawn(&mut self, kind: Kind, page: usize, points: &[PagePoint]) {
        let style = self.markbar_style(ColorTool::Shape);
        let points: Vec<[f32; 2]> = points.iter().map(|point| [point.x, point.y]).collect();
        self.emit(
            "pageDraw",
            serde_json::json!({
                "page": page,
                "kind": kind.to_string(),
                "points": points,
                "color": style.rgb(),
                "opacity": style.opacity,
            }),
        );
    }

    /// Client pixel (`x`, `y`) in page `page`'s points, on the page or off it.
    fn point_on(&self, page: usize, x: i32, y: i32) -> Option<PagePoint> {
        let (lx, ly) = self.viewport.to_layout_point(x as f32, y as f32);
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        Some(placement.to_page(lx, ly))
    }

    /// How far client pixel (`x`, `y`) is from `point` on page `page`, in pixels.
    fn screen_distance(&self, page: usize, point: PagePoint, x: i32, y: i32) -> f32 {
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return f32::INFINITY;
        };
        let (lx, ly) = placement.to_layout(point);
        let (sx, sy) = self.viewport.to_screen_point(lx, ly);
        (sx - x as f32).hypot(sy - y as f32)
    }

    /// Where the drag ends with the mouse at client pixel (`x`, `y`), in its page's points: while Shift is down, a
    /// line snapped and a box made square.
    fn drag_end(&self, drag: Drag, x: i32, y: i32) -> Option<PagePoint> {
        let to = self.point_on(drag.page, x, y)?;
        Some(match self.drawing.tool {
            Some(kind) if self.host.shift_down() => shapes::constrain(kind, drag.from, to),
            _ => to,
        })
    }

    /// Where a clicked shape's next corner goes with the mouse at client pixel (`x`, `y`): its side from the last
    /// corner snapped while Shift is down.
    fn trace_point(&self, trace: &Trace, x: i32, y: i32) -> Option<PagePoint> {
        let at = self.point_on(trace.page, x, y)?;
        Some(match trace.points.last() {
            Some(&last) if self.host.shift_down() => shapes::snap(last, at),
            _ => at,
        })
    }

    /// Draws the shape being drawn: a dragged one's outline and an arrow's head, or a clicked one's sides so far
    /// and the next to the mouse.
    pub(super) fn draw_shape_preview(&mut self) -> Result<(), H::Error> {
        let Some(kind) = self.drawing.tool else {
            return Ok(());
        };
        let (page, points, closing) = match (&self.drawing.trace, self.drawing.drag) {
            (Some(trace), _) => {
                let mut points = trace.points.clone();
                points.extend(trace.next);
                let closing = (kind.is_closed() && points.len() > 2)
                    .then(|| vec![*points.last().expect("more than two"), trace.points[0]]);
                (trace.page, points, closing)
            }
            (None, Some(drag)) => (drag.page, shapes::outline(kind, drag.from, drag.to), None),
            (None, None) => return Ok(()),
        };
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return Ok(());
        };
        let width = shapes::WIDTH * self.viewport.scale();
        let screen = |point: &PagePoint| {
            let (x, y) = placement.to_layout(*point);
            let (x, y) = self.viewport.to_screen_point(x, y);
            (x, y, width)
        };
        let mut strokes: Vec<Vec<InkPixel>> = vec![points.iter().map(screen).collect()];
        // The side that closes a polygon, thinner: it is drawn for the reader, not yet put down.
        if let Some(closing) = closing {
            strokes.push(
                closing
                    .iter()
                    .map(|point| {
                        let (x, y, _) = screen(point);
                        (x, y, width / 2.0)
                    })
                    .collect(),
            );
        }
        if let (Some(drag), LineEnd::OpenArrow) = (self.drawing.drag, kind.ends()[1]) {
            let (from, to) = (screen(&drag.from), screen(&drag.to));
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx.hypot(dy);
            if length > 0.0 {
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
                strokes.push(vec![side(HEAD_ANGLE), to, side(-HEAD_ANGLE)]);
            }
        }
        let color = self.markbar_style(ColorTool::Shape).color;
        for stroke in &strokes {
            self.host.polyline(stroke, color)?;
        }
        Ok(())
    }
}
