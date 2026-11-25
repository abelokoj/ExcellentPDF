//! Selected annotations on the pages (A.13): the app says which are selected and where their grips are (the
//! `selected` prop); the canvas outlines them, draws their grips, and lets a drag move one, resize it from a corner or
//! move one of its corners, showing the outline as it goes. When the button comes up, the drag goes to the app
//! (`annotDrag`: `{page, id, grip, index, from: [x, y], to: [x, y]}`, page points), which makes the edit. A drag that
//! starts on the page away from text and from the selection draws a marquee (`annotMarquee`: `{page, rect: [x0, y0,
//! x1, y1], add}`); a short one is a click. With the lasso on (H.6, the `lasso` prop) the drag draws a freeform loop
//! instead (`annotLasso`: `{page, path: [[x, y]], add}`). The models are `excellent-features`' `annotate::edit` and
//! `annotate::lasso`.

use excellent_features::annotate::edit::Grip;
use excellent_mupdf::{PagePoint, PageRect};
use serde_json::Value;

use super::PageCanvas;
use crate::Rect;
use crate::page::host::{Host, InkPixel};
use crate::page::placement::Placement;

/// The selection's outline and grips.
const OUTLINE: u32 = 0x2f6fde;
/// Inside a grip.
const GRIP_FILL: u32 = 0xffffff;
/// A grip's half width, and how near the pointer must come to take hold of one, in pixels.
const GRIP: f32 = 4.0;
const REACH: f32 = 7.0;
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
    /// Whether the pointer is over a grip or an item it would move, for the cursor.
    hover: bool,
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

    /// Whether the pointer would take hold of the selection, or has, for the cursor.
    pub(super) fn over_selected(&self) -> bool {
        self.selected.drag.is_some() || self.selected.hover
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
        let hover = at.is_some_and(|(x, y)| self.grip_at(x, y).is_some());
        self.selected.hover = hover;
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

    /// The selected item and grip at client pixel (`x`, `y`): a grip within reach first, then the inside of an
    /// unlocked item's bounds, which moves it.
    fn grip_at(&self, x: i32, y: i32) -> Option<(usize, Grip)> {
        let page = self.selected.page;
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
                .find(|(_, point)| self.screen_distance(page, *point, x, y) <= REACH)
                .map(|(grip, _)| (at, *grip))
        });
        grip.or_else(|| {
            let point = self.point_on(page, x, y)?;
            let slop = REACH / self.viewport.scale().max(f32::EPSILON);
            movable()
                .find(|(_, item)| {
                    let b = item.bounds;
                    (b.x0 - slop..=b.x1 + slop).contains(&point.x)
                        && (b.y0 - slop..=b.y1 + slop).contains(&point.y)
                })
                .map(|(at, _)| (at, Grip::Move))
        })
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

    /// A grip at `point` on page `page`: a small white square ringed in the outline's colour.
    fn grip(&mut self, page: usize, point: PagePoint) -> Result<(), H::Error> {
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return Ok(());
        };
        let (lx, ly) = placement.to_layout(point);
        let (x, y) = self.viewport.to_screen_point(lx, ly);
        let square = |half: f32| Rect {
            x0: x - half,
            y0: y - half,
            x1: x + half,
            y1: y + half,
        };
        self.host.fill(&square(GRIP), OUTLINE)?;
        self.host.fill(&square(GRIP - 1.5), GRIP_FILL)
    }
}

/// Where `item`'s bounds would be after `drag`: moved, or with the dragged corner or vertex at the mouse.
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
    }
}
