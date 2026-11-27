//! Selected annotations on the pages (A.13): the app says which are selected and where their grips are (the
//! `selected` prop); the canvas outlines them, draws their grips, and lets a drag move one, resize it from a corner or
//! move one of its corners, showing the outline as it goes. When the button comes up, the drag goes to the app
//! (`annotDrag`: `{page, id, grip, index, from: [x, y], to: [x, y]}`, page points), which makes the edit. A drag that
//! starts on the page away from text and from the selection draws a marquee (`annotMarquee`: `{page, rect: [x0, y0,
//! x1, y1], add}`); a short one is a click. With the lasso on (H.6, the `lasso` prop) the drag draws a freeform loop
//! instead (`annotLasso`: `{page, path: [[x, y]], add}`). The models are `ecclesia-features`' `annotate::edit` and
//! `annotate::lasso`.

use ecclesia_core::transform::Rotation;
use ecclesia_features::annotate::edit::Grip;
use ecclesia_mupdf::{PagePoint, PageRect};
use serde_json::Value;

use super::{Cursor, PageCanvas};
use crate::Rect;
use crate::page::host::{Host, InkPixel};
use crate::page::placement::Placement;

/// The selection's outline and grips.
const OUTLINE: u32 = 0x2f6fde;
/// Inside a grip.
const GRIP_FILL: u32 = 0xffffff;
/// A grip's half width, handle reach, and side-edge grab distance at 96 dpi.
const GRIP: f32 = 4.5;
const REACH: f32 = 9.0;
const EDGE_REACH: f32 = 6.0;
/// A drag shorter than this, in pixels, is a click.
const CLICK: f32 = 4.0;

/// One selected annotation, as the app describes it.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    id: String,
    bounds: PageRect,
    grips: Vec<(Grip, PagePoint)>,
    /// A locked one is outlined but can't be dragged.
    locked: bool,
}

/// The selection, and a drag on it or a marquee under way.
#[derive(Debug, Default)]
pub(super) struct Selected {
    page: usize,
    items: Vec<Item>,
    drag: Option<Drag>,
    marquee: Option<Marquee>,
    /// The lasso is on (H.6): a drag on a page draws a loop rather than a marquee.
    lasso: bool,
    /// Snapshot mode is on (O.23): a drag on a page marks a region to render, on any document.
    snapshot: bool,
    /// A loop under way.
    looping: Option<Loop>,
    /// Which grip the pointer is currently hovering over, for directional resize / move cursors.
    hover: Option<Grip>,
}

/// A drag on item `item`'s `grip`, from where the button went down to where the mouse is, page points.
#[derive(Debug, Clone, Copy)]
struct Drag {
    item: usize,
    grip: Grip,
    from: PagePoint,
    to: PagePoint,
    /// Where it started, client pixels: a short drag is a click.
    start: (i32, i32),
    moved: bool,
}

/// A lasso's loop on page `page`, page points, as drawn.
#[derive(Debug, Clone)]
struct Loop {
    page: usize,
    points: Vec<PagePoint>,
    start: (i32, i32),
    moved: bool,
    add: bool,
}

/// A marquee on page `page`, page points.
#[derive(Debug, Clone, Copy)]
struct Marquee {
    page: usize,
    from: PagePoint,
    to: PagePoint,
    start: (i32, i32),
    moved: bool,
    add: bool,
}

impl<H: Host> PageCanvas<H> {
    /// The app's selection (the `selected` prop: `{page, items: [{id, bounds: [x0, y0, x1, y1], grips: [[grip,
    /// index, x, y]], locked}]}`); anything else selects nothing. A drag under way is dropped when it changes.
    pub(crate) fn set_selected(&mut self, value: Option<&Value>) {
        let (page, items) = value.and_then(parse).unwrap_or_default();
        if self.selected.page != page || self.selected.items != items {
            self.selected.page = page;
            self.selected.items = items;
            self.selected.drag = None;
            self.invalidate();
        }
    }

    /// Whether a drag on a page draws a lasso's loop rather than a marquee (the `lasso` prop, H.6).
    pub(crate) fn set_lasso(&mut self, on: bool) {
        self.selected.lasso = on;
        if !on && self.selected.looping.take().is_some() {
            self.invalidate();
        }
    }

    /// Whether the lasso or snapshot mode is on: the pen leaves its strokes to the mouse's handling, which draws the
    /// loop or the region.
    pub(super) fn lasso_on(&self) -> bool {
        self.selected.lasso || self.selected.snapshot
    }

    /// Whether a drag on a page marks a region to snapshot (the `snapshot` prop, O.23).
    pub(crate) fn set_snapshot(&mut self, on: bool) {
        self.selected.snapshot = on;
    }

    /// Pointer cursor shape for the active selection drag or hovered grip, if any.
    pub(super) fn selected_cursor(&self) -> Option<Cursor> {
        let grip = self
            .selected
            .drag
            .map(|drag| drag.grip)
            .or(self.selected.hover)?;
        Some(grip_cursor(grip, self.view.rotation))
    }

    /// Pointer cursor shape evaluated live at client pixel (`x`, `y`) over the selection.
    pub(super) fn selected_cursor_at(&self, x: i32, y: i32) -> Option<Cursor> {
        if let Some(drag) = self.selected.drag {
            return Some(grip_cursor(drag.grip, self.view.rotation));
        }
        let (_, grip) = self.grip_at(x, y)?;
        Some(grip_cursor(grip, self.view.rotation))
    }

    /// Page, id and bounds of the single selected annotation when exactly one annotation is
    /// selected and no drag is under way, for anchoring the external floating selection popup.
    pub(super) fn single_selected_bounds(&self) -> Option<(usize, &str, PageRect)> {
        if self.selected.drag.is_some() || self.selected.items.len() != 1 {
            return None;
        }
        let item = self.selected.items.first()?;
        Some((self.selected.page, item.id.as_str(), item.bounds))
    }

    /// The left button went down at client pixel (`x`, `y`): on a selected annotation's grip, or inside one, a drag
    /// starts. Returns whether it did, when the window should capture the mouse.
    pub(super) fn select_down(&mut self, x: i32, y: i32) -> bool {
        let Some((item, grip)) = self.grip_at(x, y) else {
            return false;
        };
        let Some(from) = self.point_on(self.selected.page, x, y) else {
            return false;
        };
        self.selected.drag = Some(Drag {
            item,
            grip,
            from,
            to: from,
            start: (x, y),
            moved: false,
        });
        true
    }

    /// The left button went down at client pixel (`x`, `y`) on a page, away from text and links: a marquee starts,
    /// when the document can take annotations. Returns whether it did.
    pub(super) fn marquee_down(&mut self, x: i32, y: i32) -> bool {
        // A snapshot is of any document; selecting annotations needs one that takes them.
        if !(self.selected.snapshot || self.can_mark()) || self.reflow.is_some() {
            return false;
        }
        let Some((page, from)) = self.page_point(x, y) else {
            return false;
        };
        if self.selected.lasso && !self.selected.snapshot {
            self.selected.looping = Some(Loop {
                page,
                points: vec![from],
                start: (x, y),
                moved: false,
                add: self.host.ctrl_down(),
            });
            return true;
        }
        self.selected.marquee = Some(Marquee {
            page,
            from,
            to: from,
            start: (x, y),
            moved: false,
            add: self.host.ctrl_down(),
        });
        true
    }

    /// The mouse moved to client pixel (`x`, `y`) (`None`: it left): a drag or a marquee follows it. Returns
    /// whether one is under way.
    pub(super) fn select_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        let far = |start: (i32, i32), (x, y): (i32, i32)| {
            ((x - start.0) as f32).hypot((y - start.1) as f32) >= CLICK
        };
        if let Some(mut drag) = self.selected.drag {
            if let Some((x, y)) = at
                && let Some(to) = self.point_on(self.selected.page, x, y)
            {
                drag.moved |= far(drag.start, (x, y));
                drag.to = to;
                self.selected.drag = Some(drag);
                self.invalidate();
            }
            return true;
        }
        if let Some(mut marquee) = self.selected.marquee {
            if let Some((x, y)) = at
                && let Some(to) = self.point_on(marquee.page, x, y)
            {
                marquee.moved |= far(marquee.start, (x, y));
                marquee.to = to;
                self.selected.marquee = Some(marquee);
                self.invalidate();
            }
            return true;
        }
        if let Some(mut looping) = self.selected.looping.take() {
            if let Some((x, y)) = at
                && let Some(to) = self.point_on(looping.page, x, y)
            {
                looping.moved |= far(looping.start, (x, y));
                looping.points.push(to);
                self.invalidate();
            }
            self.selected.looping = Some(looping);
            return true;
        }
        self.selected.hover = at.and_then(|(x, y)| self.grip_at(x, y).map(|(_, grip)| grip));
        false
    }

    /// The left button came up at client pixel (`x`, `y`): a drag goes to the app, or a marquee does; a short one
    /// is a click on the page (`pagePress`). Returns whether either was under way.
    pub(super) fn select_up(&mut self, x: i32, y: i32) -> bool {
        if let Some(drag) = self.selected.drag.take() {
            self.invalidate();
            if !drag.moved {
                self.page_press(x, y);
                return true;
            }
            let to = self.point_on(self.selected.page, x, y).unwrap_or(drag.to);
            if let Some(item) = self.selected.items.get(drag.item) {
                self.emit(
                    "annotDrag",
                    serde_json::json!({
                        "page": self.selected.page,
                        "id": item.id,
                        "grip": drag.grip.name(),
                        "index": drag.grip.index(),
                        "from": [drag.from.x, drag.from.y],
                        "to": [to.x, to.y],
                    }),
                );
            }
            return true;
        }
        if let Some(looping) = self.selected.looping.take() {
            self.invalidate();
            if !looping.moved || looping.points.len() < 3 {
                self.page_press(x, y);
                return true;
            }
            let path: Vec<[f32; 2]> = looping.points.iter().map(|p| [p.x, p.y]).collect();
            self.emit(
                "annotLasso",
                serde_json::json!({ "page": looping.page, "path": path, "add": looping.add }),
            );
            return true;
        }
        if let Some(marquee) = self.selected.marquee.take() {
            self.invalidate();
            if !marquee.moved {
                self.page_press(x, y);
                return true;
            }
            let to = self.point_on(marquee.page, x, y).unwrap_or(marquee.to);
            if self.selected.snapshot {
                self.emit(
                    "snapshot",
                    serde_json::json!({
                        "page": marquee.page,
                        "rect": [marquee.from.x, marquee.from.y, to.x, to.y],
                    }),
                );
                return true;
            }
            self.emit(
                "annotMarquee",
                serde_json::json!({
                    "page": marquee.page,
                    "rect": [marquee.from.x, marquee.from.y, to.x, to.y],
                    "add": marquee.add,
                }),
            );
            return true;
        }
        false
    }

    /// Something else took the mouse: a drag or a marquee is dropped.
    pub(super) fn select_cancel(&mut self) {
        if self.selected.drag.take().is_some()
            | self.selected.marquee.take().is_some()
            | self.selected.looping.take().is_some()
        {
            self.invalidate();
        }
    }

    /// The selected item and grip at client pixel (`x`, `y`): explicit corner/edge handles within reach first,
    /// then any border side of an edge-resizable item (e.g. a block highlight), then the inside of an unlocked
    /// item's bounds, which moves it.
    fn grip_at(&self, x: i32, y: i32) -> Option<(usize, Grip)> {
        let page = self.selected.page;
        let scale = self.dpi_scale().max(1.0);
        let reach = REACH * scale;
        let edge_reach = EDGE_REACH * scale;
        let movable = || {
            self.selected
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.locked)
        };
        let grip = movable().find_map(|(at, item)| {
            item.grips
                .iter()
                .find(|(_, point)| self.screen_distance(page, *point, x, y) <= reach)
                .map(|(grip, _)| (at, *grip))
        });
        if grip.is_some() {
            return grip;
        }
        if let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        {
            let px = x as f32;
            let py = y as f32;
            let to_screen = |pt: PagePoint| {
                let (lx, ly) = placement.to_layout(pt);
                self.viewport.to_screen_point(lx, ly)
            };
            let edge_hit = movable().find_map(|(at, item)| {
                if !item
                    .grips
                    .iter()
                    .any(|(grip, _)| matches!(grip, Grip::Edge(_)))
                {
                    return None;
                }
                let b = item.bounds;
                let tl = to_screen(PagePoint { x: b.x0, y: b.y0 });
                let tr = to_screen(PagePoint { x: b.x1, y: b.y0 });
                let br = to_screen(PagePoint { x: b.x1, y: b.y1 });
                let bl = to_screen(PagePoint { x: b.x0, y: b.y1 });
                let sides = [
                    (Grip::Edge(0), tl, tr),
                    (Grip::Edge(1), tr, br),
                    (Grip::Edge(2), br, bl),
                    (Grip::Edge(3), bl, tl),
                ];
                sides
                    .into_iter()
                    .filter_map(|(grip, a, b_pt)| {
                        let dist = point_to_segment_distance(px, py, a, b_pt);
                        (dist <= edge_reach).then_some((grip, dist))
                    })
                    .min_by(|a, b_cmp| a.1.total_cmp(&b_cmp.1))
                    .map(|(grip, _)| (at, grip))
            });
            if edge_hit.is_some() {
                return edge_hit;
            }
        }
        let point = self.point_on(page, x, y)?;
        let slop = reach / self.viewport.scale().max(f32::EPSILON);
        movable()
            .find(|(_, item)| {
                let b = item.bounds;
                let has_edges = item
                    .grips
                    .iter()
                    .any(|(grip, _)| matches!(grip, Grip::Edge(_)));
                let pad = if has_edges { 0.0 } else { slop };
                (b.x0 - pad..=b.x1 + pad).contains(&point.x)
                    && (b.y0 - pad..=b.y1 + pad).contains(&point.y)
            })
            .map(|(at, _)| (at, Grip::Move))
    }

    /// Draws the selection: each item's outline and grips, a drag's outline where it would go, and a marquee.
    pub(super) fn draw_selected(&mut self) -> Result<(), H::Error> {
        if let Some(looping) = self.selected.looping.as_ref().filter(|l| l.moved) {
            let (page, points) = (looping.page, looping.points.clone());
            self.trace(page, &points)?;
        }
        if let Some(marquee) = self.selected.marquee.filter(|m| m.moved) {
            let rect = PageRect {
                x0: marquee.from.x,
                y0: marquee.from.y,
                x1: marquee.to.x,
                y1: marquee.to.y,
            };
            self.outline(marquee.page, rect)?;
        }
        let page = self.selected.page;
        let drag = self.selected.drag.filter(|drag| drag.moved);
        for (at, item) in self.selected.items.clone().iter().enumerate() {
            // A move takes every unlocked one along, as the app will (H.6: a lasso's selection moves whole).
            let bounds = match drag {
                Some(drag) if drag.item == at => dragged_bounds(item, drag),
                Some(drag) if drag.grip == Grip::Move && !item.locked => dragged_bounds(item, drag),
                _ => item.bounds,
            };
            self.outline(page, bounds)?;
            if item.locked || drag.is_some() {
                continue;
            }
            for (_, point) in &item.grips {
                self.grip(page, *point)?;
            }
        }
        Ok(())
    }

    /// The loop through `points` on page `page`, closed back to its start.
    fn trace(&mut self, page: usize, points: &[PagePoint]) -> Result<(), H::Error> {
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return Ok(());
        };
        let ring: Vec<InkPixel> = points
            .iter()
            .chain(points.first())
            .map(|&p| {
                let (lx, ly) = placement.to_layout(p);
                let (sx, sy) = self.viewport.to_screen_point(lx, ly);
                (sx, sy, 1.5)
            })
            .collect();
        self.host.polyline(&ring, OUTLINE)
    }

    /// `rect` on page `page`, outlined.
    fn outline(&mut self, page: usize, rect: PageRect) -> Result<(), H::Error> {
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return Ok(());
        };
        let screen = |x: f32, y: f32| {
            let (lx, ly) = placement.to_layout(PagePoint { x, y });
            let (sx, sy) = self.viewport.to_screen_point(lx, ly);
            (sx, sy, 1.5)
        };
        let ring: Vec<InkPixel> = vec![
            screen(rect.x0, rect.y0),
            screen(rect.x1, rect.y0),
            screen(rect.x1, rect.y1),
            screen(rect.x0, rect.y1),
            screen(rect.x0, rect.y0),
        ];
        self.host.polyline(&ring, OUTLINE)
    }

    /// A grip at `point` on page `page`: a crisp white square ringed in the outline's colour, scaled for DPI.
    fn grip(&mut self, page: usize, point: PagePoint) -> Result<(), H::Error> {
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return Ok(());
        };
        let scale = self.dpi_scale().max(1.0);
        let half = GRIP * scale;
        let border = 1.5 * scale;
        let (lx, ly) = placement.to_layout(point);
        let (x, y) = self.viewport.to_screen_point(lx, ly);
        let square = |h: f32| Rect {
            x0: x - h,
            y0: y - h,
            x1: x + h,
            y1: y + h,
        };
        self.host.fill(&square(half), OUTLINE)?;
        self.host.fill(&square((half - border).max(1.0)), GRIP_FILL)
    }
}

/// Maps `grip` to the directional pointer cursor shape, accounting for page `rotation`.
fn grip_cursor(grip: Grip, rotation: Rotation) -> Cursor {
    let quarter_turned = matches!(rotation, Rotation::Clockwise | Rotation::Anticlockwise);
    match grip {
        Grip::Corner(0 | 2) => {
            if quarter_turned {
                Cursor::ResizeNesw
            } else {
                Cursor::ResizeNwse
            }
        }
        Grip::Corner(1 | 3) => {
            if quarter_turned {
                Cursor::ResizeNwse
            } else {
                Cursor::ResizeNesw
            }
        }
        Grip::Edge(0 | 2) => {
            if quarter_turned {
                Cursor::ResizeEw
            } else {
                Cursor::ResizeNs
            }
        }
        Grip::Edge(1 | 3) => {
            if quarter_turned {
                Cursor::ResizeNs
            } else {
                Cursor::ResizeEw
            }
        }
        Grip::Move | Grip::Corner(_) | Grip::Edge(_) | Grip::Vertex(_) => Cursor::Pan,
    }
}

/// Shortest Euclidean distance from `(px, py)` to the line segment `a`–`b`.
fn point_to_segment_distance(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f32::EPSILON {
        return (px - a.0).hypot(py - a.1);
    }
    let t = (((px - a.0) * dx + (py - a.1) * dy) / len_sq).clamp(0.0, 1.0);
    let proj_x = a.0 + t * dx;
    let proj_y = a.1 + t * dy;
    (px - proj_x).hypot(py - proj_y)
}

/// Where `item`'s bounds would be after `drag`: moved, or resized from a corner, edge or vertex.
fn dragged_bounds(item: &Item, drag: Drag) -> PageRect {
    let b = item.bounds;
    let (dx, dy) = (drag.to.x - drag.from.x, drag.to.y - drag.from.y);
    match drag.grip {
        Grip::Move => PageRect {
            x0: b.x0 + dx,
            y0: b.y0 + dy,
            x1: b.x1 + dx,
            y1: b.y1 + dy,
        },
        Grip::Edge(0) => PageRect {
            x0: b.x0,
            y0: drag.to.y.min(b.y1),
            x1: b.x1,
            y1: drag.to.y.max(b.y1),
        },
        Grip::Edge(1) => PageRect {
            x0: drag.to.x.min(b.x0),
            y0: b.y0,
            x1: drag.to.x.max(b.x0),
            y1: b.y1,
        },
        Grip::Edge(2) => PageRect {
            x0: b.x0,
            y0: drag.to.y.min(b.y0),
            x1: b.x1,
            y1: drag.to.y.max(b.y0),
        },
        Grip::Edge(3) => PageRect {
            x0: drag.to.x.min(b.x1),
            y0: b.y0,
            x1: drag.to.x.max(b.x1),
            y1: b.y1,
        },
        Grip::Edge(_) => b,
        Grip::Corner(_) | Grip::Vertex(_) => {
            // The other grips stay; the bounds take in the dragged one where the mouse is.
            let others = item
                .grips
                .iter()
                .filter(|(grip, _)| *grip != drag.grip)
                .map(|(_, point)| *point);
            let kept: Vec<PagePoint> = match drag.grip {
                Grip::Corner(index) => item
                    .grips
                    .iter()
                    .find(|(grip, _)| *grip == Grip::Corner((index + 2) % 4))
                    .map(|(_, point)| vec![*point])
                    .unwrap_or_default(),
                _ => others.collect(),
            };
            kept.into_iter().fold(
                PageRect {
                    x0: drag.to.x,
                    y0: drag.to.y,
                    x1: drag.to.x,
                    y1: drag.to.y,
                },
                |r, p| PageRect {
                    x0: r.x0.min(p.x),
                    y0: r.y0.min(p.y),
                    x1: r.x1.max(p.x),
                    y1: r.y1.max(p.y),
                },
            )
        }
    }
}

/// The `selected` prop's page and items.
fn parse(value: &Value) -> Option<(usize, Vec<Item>)> {
    let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
    let number = |value: &Value| value.as_f64().map(|n| n as f32);
    let items = value
        .get("items")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            let bounds: Vec<f32> = item
                .get("bounds")?
                .as_array()?
                .iter()
                .filter_map(number)
                .collect();
            let [x0, y0, x1, y1] = <[f32; 4]>::try_from(bounds).ok()?;
            let grips = item
                .get("grips")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|grip| {
                    let grip = grip.as_array()?;
                    let index = usize::try_from(grip.get(1)?.as_u64()?).ok()?;
                    let kind = Grip::parse(grip.first()?.as_str()?, index)?;
                    Some((
                        kind,
                        PagePoint {
                            x: number(grip.get(2)?)?,
                            y: number(grip.get(3)?)?,
                        },
                    ))
                })
                .collect();
            Some(Item {
                id: item.get("id")?.as_str()?.to_owned(),
                bounds: PageRect { x0, y0, x1, y1 },
                grips,
                locked: item.get("locked").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();
    Some((page, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_prop_reads_its_items_and_grips() {
        let value = serde_json::json!({
            "page": 3,
            "items": [
                {"id": "a", "bounds": [1, 2, 3, 4], "grips": [["corner", 2, 3, 4], ["bad", 0, 0, 0]], "locked": false},
                {"id": "b", "bounds": [1, 2]},
            ],
        });
        let (page, items) = parse(&value).expect("parses");
        assert_eq!(page, 3);
        assert_eq!(items.len(), 1, "an item without four bounds is left out");
        assert_eq!(
            items[0].grips,
            vec![(Grip::Corner(2), PagePoint { x: 3.0, y: 4.0 })]
        );
    }

    #[test]
    fn a_drag_s_outline_follows_the_grip() {
        let item = Item {
            id: "a".to_owned(),
            bounds: PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: 10.0,
            },
            grips: vec![
                (Grip::Corner(0), PagePoint { x: 0.0, y: 0.0 }),
                (Grip::Corner(2), PagePoint { x: 10.0, y: 10.0 }),
            ],
            locked: false,
        };
        let drag = |grip, to| Drag {
            item: 0,
            grip,
            from: PagePoint { x: 10.0, y: 10.0 },
            to,
            start: (0, 0),
            moved: true,
        };
        let moved = dragged_bounds(&item, drag(Grip::Move, PagePoint { x: 15.0, y: 10.0 }));
        assert_eq!((moved.x0, moved.x1), (5.0, 15.0));
        let grown = dragged_bounds(&item, drag(Grip::Corner(2), PagePoint { x: 20.0, y: 30.0 }));
        assert_eq!(
            (grown.x0, grown.y0, grown.x1, grown.y1),
            (0.0, 0.0, 20.0, 30.0)
        );
        let top = dragged_bounds(&item, drag(Grip::Edge(0), PagePoint { x: 8.0, y: -5.0 }));
        assert_eq!((top.x0, top.y0, top.x1, top.y1), (0.0, -5.0, 10.0, 10.0));
        let right = dragged_bounds(&item, drag(Grip::Edge(1), PagePoint { x: 25.0, y: 3.0 }));
        assert_eq!(
            (right.x0, right.y0, right.x1, right.y1),
            (0.0, 0.0, 25.0, 10.0)
        );
        let bottom = dragged_bounds(&item, drag(Grip::Edge(2), PagePoint { x: 4.0, y: 18.0 }));
        assert_eq!(
            (bottom.x0, bottom.y0, bottom.x1, bottom.y1),
            (0.0, 0.0, 10.0, 18.0)
        );
        let left = dragged_bounds(&item, drag(Grip::Edge(3), PagePoint { x: -4.0, y: 7.0 }));
        assert_eq!(
            (left.x0, left.y0, left.x1, left.y1),
            (-4.0, 0.0, 10.0, 10.0)
        );
    }

    #[test]
    fn grip_cursors_reflect_all_eight_directions_and_page_rotation() {
        assert_eq!(
            grip_cursor(Grip::Corner(0), Rotation::Upright),
            Cursor::ResizeNwse
        );
        assert_eq!(
            grip_cursor(Grip::Corner(2), Rotation::Upright),
            Cursor::ResizeNwse
        );
        assert_eq!(
            grip_cursor(Grip::Corner(1), Rotation::Upright),
            Cursor::ResizeNesw
        );
        assert_eq!(
            grip_cursor(Grip::Corner(3), Rotation::Upright),
            Cursor::ResizeNesw
        );
        assert_eq!(
            grip_cursor(Grip::Edge(0), Rotation::Upright),
            Cursor::ResizeNs
        );
        assert_eq!(
            grip_cursor(Grip::Edge(2), Rotation::Upright),
            Cursor::ResizeNs
        );
        assert_eq!(
            grip_cursor(Grip::Edge(1), Rotation::Upright),
            Cursor::ResizeEw
        );
        assert_eq!(
            grip_cursor(Grip::Edge(3), Rotation::Upright),
            Cursor::ResizeEw
        );
        assert_eq!(grip_cursor(Grip::Move, Rotation::Upright), Cursor::Pan);

        assert_eq!(
            grip_cursor(Grip::Corner(0), Rotation::Clockwise),
            Cursor::ResizeNesw
        );
        assert_eq!(
            grip_cursor(Grip::Corner(1), Rotation::Clockwise),
            Cursor::ResizeNwse
        );
        assert_eq!(
            grip_cursor(Grip::Edge(0), Rotation::Clockwise),
            Cursor::ResizeEw
        );
        assert_eq!(
            grip_cursor(Grip::Edge(1), Rotation::Clockwise),
            Cursor::ResizeNs
        );

        assert!((point_to_segment_distance(5.0, 3.0, (0.0, 0.0), (10.0, 0.0)) - 3.0).abs() < 1e-4);
        assert!((point_to_segment_distance(14.0, 3.0, (0.0, 0.0), (10.0, 0.0)) - 5.0).abs() < 1e-4);
    }
}
