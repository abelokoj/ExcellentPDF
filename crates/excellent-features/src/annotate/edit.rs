//! Selecting and editing annotations on the page (A.13): where a selected annotation's grips are, what dragging one
//! of them does (move it, resize it from a corner, move one of its corners), nudging, which annotations a marquee
//! takes, and the annotation clipboard that copies between documents. Pure: the canvas draws the grips and reports
//! the drags, the app makes each edit one change for undo. A locked annotation (A.12) keeps still: it has no grips
//! and its edits are none.

use excellent_core::anchor::new_id;
use excellent_core::annot::{Annotation, Boxed, Shape, now};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect, Quad};
use serde_json::{Value, json};

use super::inspector::allows;
use super::margin;
use super::notes::Target;

/// What a drag on a selected annotation takes hold of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    /// The whole annotation: it moves.
    Move,
    /// A corner of its bounds, clockwise from the top left: it is resized, the opposite corner kept.
    Corner(usize),
    /// One of a line's ends or a polygon's corners: that point moves.
    Vertex(usize),
}

impl Grip {
    /// Its name in the canvas's events: `move`, `corner` or `vertex`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Corner(_) => "corner",
            Self::Vertex(_) => "vertex",
        }
    }

    /// Its index: a corner's or a vertex's; 0 for a move.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Move => 0,
            Self::Corner(index) | Self::Vertex(index) => index,
        }
    }

    /// The grip named `name` with `index`.
    #[must_use]
    pub fn parse(name: &str, index: usize) -> Option<Self> {
        match name {
            "move" => Some(Self::Move),
            "corner" if index < 4 => Some(Self::Corner(index)),
            "vertex" => Some(Self::Vertex(index)),
            _ => None,
        }
    }
}

/// A drag the page canvas reports on the selection (its `annotDrag` event: `{page, id, grip, index, from: [x, y],
/// to: [x, y]}`, page points).
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    /// The page the selection is on.
    pub page: usize,
    /// The annotation dragged.
    pub id: String,
    /// What of it was dragged.
    pub grip: Grip,
    /// Where the drag started and ended.
    pub from: PagePoint,
    /// As `from`.
    pub to: PagePoint,
}

impl Drag {
    /// The drag `data` describes; `None` when it doesn't parse.
    #[must_use]
    pub fn parse(data: &Value) -> Option<Self> {
        let index = usize::try_from(data.get("index")?.as_u64()?).ok()?;
        Some(Self {
            page: usize::try_from(data.get("page")?.as_u64()?).ok()?,
            id: data.get("id")?.as_str()?.to_owned(),
            grip: Grip::parse(data.get("grip")?.as_str()?, index)?,
            from: point(data.get("from")?)?,
            to: point(data.get("to")?)?,
        })
    }

    /// The drag `annotation` takes: moved along with the one dragged, or reshaped when it is the one; `None` when it
    /// is neither, or can't be dragged so.
    #[must_use]
    pub fn applied(&self, annotation: &Annotation, selected: bool) -> Option<Annotation> {
        match self.grip {
            Grip::Move if selected || annotation.id == self.id => {
                moved(annotation, self.to.x - self.from.x, self.to.y - self.from.y)
            }
            _ if annotation.id == self.id => dragged(annotation, self.grip, self.from, self.to),
            _ => None,
        }
    }
}

/// A marquee the page canvas reports (its `annotMarquee` event: `{page, rect: [x0, y0, x1, y1], add}`, page
/// points; `add` when Ctrl was down).
#[derive(Debug, Clone, PartialEq)]
pub struct Marquee {
    /// The page it was drawn on.
    pub page: usize,
    /// What it covers, either way round.
    pub rect: PageRect,
    /// Whether it adds to the selection.
    pub add: bool,
}

impl Marquee {
    /// The marquee `data` describes; `None` when it doesn't parse.
    #[must_use]
    pub fn parse(data: &Value) -> Option<Self> {
        let numbers: Vec<f32> = data
            .get("rect")?
            .as_array()?
            .iter()
            .filter_map(|n| n.as_f64().map(|n| n as f32))
            .collect();
        let [x0, y0, x1, y1] = <[f32; 4]>::try_from(numbers).ok()?;
        Some(Self {
            page: usize::try_from(data.get("page")?.as_u64()?).ok()?,
            rect: PageRect { x0, y0, x1, y1 },
            add: data.get("add").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// The annotations of `annotations` it takes: those that lie wholly inside it.
    #[must_use]
    pub fn takes(&self, annotations: &[Annotation]) -> Vec<Target> {
        in_marquee(annotations, self.rect)
    }
}

/// `[x, y]` as a point.
fn point(value: &Value) -> Option<PagePoint> {
    let pair = value.as_array()?;
    Some(PagePoint {
        x: pair.first()?.as_f64()? as f32,
        y: pair.get(1)?.as_f64()? as f32,
    })
}

/// The annotation a click at `point` selects: the one whose note it would open (`notes::note_at`), else the last
/// drawn whose bounds hold it (an ink drawing, a stamp).
#[must_use]
pub fn hit(annotations: &[Annotation], point: PagePoint) -> Option<&Annotation> {
    super::notes::note_at(annotations, point).or_else(|| {
        annotations.iter().rev().find(|annotation| {
            !matches!(annotation.shape, Shape::Markup(..))
                && bounds(annotation).is_some_and(|b| {
                    (b.x0..=b.x1).contains(&point.x) && (b.y0..=b.y1).contains(&point.y)
                })
        })
    })
}

/// The smallest a box is resized to, in points.
const MIN_SIZE: f32 = 4.0;
/// How far a pasted annotation sits from where it was copied, in points, so it shows as a new one.
pub const PASTE_OFFSET: f32 = 12.0;
/// How far an arrow key nudges, in points.
pub const NUDGE: f32 = 1.0;

/// Whether `annotation` can be moved: not text markup, which belongs to its text, and not locked.
#[must_use]
pub fn can_move(annotation: &Annotation) -> bool {
    !annotation.locked && !matches!(annotation.shape, Shape::Markup(..))
}

/// The smallest rectangle holding `annotation`, in page points; `None` for one with no points.
#[must_use]
pub fn bounds(annotation: &Annotation) -> Option<PageRect> {
    margin::outline(&annotation.shape).bounds()
}

/// The grips a selected `annotation` offers besides moving it, and where each is: a box's, an ink drawing's or a
/// callout's corners, a line's ends and a polygon's or polyline's corners. None when it can't move; a sticky note,
/// a typewriter's text, a stamp and a caret move but keep their size.
#[must_use]
pub fn grips(annotation: &Annotation) -> Vec<(Grip, PagePoint)> {
    if !can_move(annotation) {
        return Vec::new();
    }
    match &annotation.shape {
        Shape::Line(from, to) => vec![(Grip::Vertex(0), *from), (Grip::Vertex(1), *to)],
        Shape::Path(_, points) => points
            .iter()
            .enumerate()
            .map(|(index, point)| (Grip::Vertex(index), *point))
            .collect(),
        Shape::Boxed(Boxed::Square | Boxed::Circle | Boxed::FreeText, _)
        | Shape::Callout(..)
        | Shape::Ink(_) => bounds(annotation)
            .map(|rect| {
                corners(rect)
                    .into_iter()
                    .enumerate()
                    .map(|(index, point)| (Grip::Corner(index), point))
                    .collect()
            })
            .unwrap_or_default(),
        Shape::Boxed(..) | Shape::Markup(..) => Vec::new(),
    }
}

/// `rect`'s corners, clockwise from the top left.
pub fn corners(rect: PageRect) -> [PagePoint; 4] {
    [
        PagePoint {
            x: rect.x0,
            y: rect.y0,
        },
        PagePoint {
            x: rect.x1,
            y: rect.y0,
        },
        PagePoint {
            x: rect.x1,
            y: rect.y1,
        },
        PagePoint {
            x: rect.x0,
            y: rect.y1,
        },
    ]
}

/// `annotation` after a drag on `grip` from `from` to `to`, page points; `None` when it can't be dragged so.
#[must_use]
pub fn dragged(
    annotation: &Annotation,
    grip: Grip,
    from: PagePoint,
    to: PagePoint,
) -> Option<Annotation> {
    if !can_move(annotation) {
        return None;
    }
    let shape = match grip {
        Grip::Move => moved_shape(&annotation.shape, to.x - from.x, to.y - from.y),
        Grip::Corner(index) => {
            if !grips(annotation)
                .iter()
                .any(|(grip, _)| *grip == Grip::Corner(index))
            {
                return None;
            }
            let old = bounds(annotation)?;
            let new = resized(old, index, to);
            map_shape(&annotation.shape, |point| scale(point, old, new))
        }
        Grip::Vertex(index) => match &annotation.shape {
            Shape::Line(start, end) => match index {
                0 => Shape::Line(to, *end),
                1 => Shape::Line(*start, to),
                _ => return None,
            },
            Shape::Path(kind, points) if index < points.len() => {
                let mut points = points.clone();
                points[index] = to;
                Shape::Path(*kind, points)
            }
            _ => return None,
        },
    };
    let mut after = annotation.clone();
    after.shape = shape;
    Some(after)
}

/// `annotation` moved by (`dx`, `dy`) points; `None` when it can't move.
#[must_use]
pub fn moved(annotation: &Annotation, dx: f32, dy: f32) -> Option<Annotation> {
    can_move(annotation).then(|| {
        let mut after = annotation.clone();
        after.shape = moved_shape(&annotation.shape, dx, dy);
        after
    })
}

/// The change that makes `before` into `after`, stamped now; none when nothing changed or a lock forbids it.
#[must_use]
pub fn change(before: &Annotation, mut after: Annotation) -> Option<Change> {
    if after == *before {
        return None;
    }
    after.modified = now();
    let change = Change::UpdateAnnotation(after);
    allows(before, &change).then_some(change)
}

/// The changes that delete `annotations`, leaving the locked ones.
#[must_use]
pub fn delete(annotations: &[Annotation]) -> Vec<Change> {
    annotations
        .iter()
        .filter(|annotation| !annotation.locked)
        .map(|annotation| Change::DeleteAnnotation {
            page: annotation.page,
            id: annotation.id.clone(),
        })
        .collect()
}

/// The annotations of `annotations` a marquee over `rect` takes: those that lie wholly inside it.
#[must_use]
pub fn in_marquee(annotations: &[Annotation], rect: PageRect) -> Vec<Target> {
    let rect = normalized(rect);
    annotations
        .iter()
        .filter(|annotation| {
            bounds(annotation).is_some_and(|b| {
                b.x0 >= rect.x0 && b.y0 >= rect.y0 && b.x1 <= rect.x1 && b.y1 <= rect.y1
            })
        })
        .map(|annotation| Target {
            page: annotation.page,
            id: annotation.id.clone(),
        })
        .collect()
}

/// `selection` with `target` added, or taken out when it is in it; a target on another page starts it again.
#[must_use]
pub fn toggled(selection: &[Target], target: Target) -> Vec<Target> {
    if selection.iter().any(|t| t.page != target.page) {
        return vec![target];
    }
    if selection.contains(&target) {
        selection
            .iter()
            .filter(|t| **t != target)
            .cloned()
            .collect()
    } else {
        let mut selection = selection.to_vec();
        selection.push(target);
        selection
    }
}

/// `rect` with `x0 <= x1` and `y0 <= y1`.
fn normalized(rect: PageRect) -> PageRect {
    PageRect {
        x0: rect.x0.min(rect.x1),
        y0: rect.y0.min(rect.y1),
        x1: rect.x0.max(rect.x1),
        y1: rect.y0.max(rect.y1),
    }
}

/// `old` with corner `index` dragged to `to`, the opposite corner kept, no side shorter than [`MIN_SIZE`]. The
/// dragged corner may cross the kept one: the box flips.
pub(super) fn resized(old: PageRect, index: usize, to: PagePoint) -> PageRect {
    let kept = corners(old)[(index + 2) % 4];
    let away = |from: f32, to: f32| {
        if (to - from).abs() >= MIN_SIZE {
            to
        } else if to >= from {
            from + MIN_SIZE
        } else {
            from - MIN_SIZE
        }
    };
    normalized(PageRect {
        x0: kept.x,
        y0: kept.y,
        x1: away(kept.x, to.x),
        y1: away(kept.y, to.y),
    })
}

/// `point` in `old` mapped to the same place in `new`.
pub(super) fn scale(point: PagePoint, old: PageRect, new: PageRect) -> PagePoint {
    let along = |p: f32, a0: f32, a1: f32, b0: f32, b1: f32| {
        if (a1 - a0).abs() < f32::EPSILON {
            b0 + (p - a0)
        } else {
            b0 + (p - a0) * (b1 - b0) / (a1 - a0)
        }
    };
    PagePoint {
        x: along(point.x, old.x0, old.x1, new.x0, new.x1),
        y: along(point.y, old.y0, old.y1, new.y0, new.y1),
    }
}

/// `shape` moved by (`dx`, `dy`).
fn moved_shape(shape: &Shape, dx: f32, dy: f32) -> Shape {
    map_shape(shape, |point| PagePoint {
        x: point.x + dx,
        y: point.y + dy,
    })
}

/// `shape` with every point mapped by `map`; a box's rectangle stays upright.
pub(super) fn map_shape(shape: &Shape, map: impl Fn(PagePoint) -> PagePoint) -> Shape {
    let rect = |r: &PageRect| {
        let (a, b) = (
            map(PagePoint { x: r.x0, y: r.y0 }),
            map(PagePoint { x: r.x1, y: r.y1 }),
        );
        normalized(PageRect {
            x0: a.x,
            y0: a.y,
            x1: b.x,
            y1: b.y,
        })
    };
    match shape {
        Shape::Markup(kind, quads) => Shape::Markup(
            *kind,
            quads
                .iter()
                .map(|q| Quad {
                    ul: map(q.ul),
                    ur: map(q.ur),
                    ll: map(q.ll),
                    lr: map(q.lr),
                })
                .collect(),
        ),
        Shape::Boxed(kind, r) => Shape::Boxed(*kind, rect(r)),
        Shape::Ink(strokes) => Shape::Ink(
            strokes
                .iter()
                .map(|stroke| stroke.iter().copied().map(&map).collect())
                .collect(),
        ),
        Shape::Line(a, b) => Shape::Line(map(*a), map(*b)),
        Shape::Path(kind, points) => Shape::Path(*kind, points.iter().copied().map(&map).collect()),
        Shape::Callout(r, tip) => Shape::Callout(rect(r), map(*tip)),
    }
}

/// The annotation clipboard: what was copied, whole, to paste into this document or another.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    annotations: Vec<Annotation>,
}

impl Clip {
    /// `annotations`, copied.
    #[must_use]
    pub fn of(annotations: Vec<Annotation>) -> Self {
        Self { annotations }
    }

    /// Whether nothing was copied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    /// Its text, for other apps: each annotation's note, one a paragraph.
    #[must_use]
    pub fn text(&self) -> String {
        self.annotations
            .iter()
            .map(|annotation| annotation.contents.trim())
            .filter(|contents| !contents.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The annotations to create for a paste onto page `page`: each as copied (its kind, look, note and tags), with
    /// a new id, made now, unlocked and ungrouped, moved down and right by `offset` points when it goes on the page
    /// it was copied from. Text markup is pasted only onto its own page, where its text is.
    #[must_use]
    pub fn paste(&self, page: usize, offset: f32, author: &str) -> Vec<Annotation> {
        let made = now();
        self.annotations
            .iter()
            .filter(|annotation| {
                !matches!(annotation.shape, Shape::Markup(..)) || annotation.page == page
            })
            .map(|annotation| {
                let mut pasted = annotation.clone();
                let shift = if matches!(annotation.shape, Shape::Markup(..)) {
                    0.0
                } else if annotation.page == page {
                    offset
                } else {
                    0.0
                };
                pasted.shape = moved_shape(&annotation.shape, shift, shift);
                pasted.id = new_id();
                pasted.page = page;
                pasted.created = made;
                pasted.modified = made;
                pasted.group = None;
                pasted.locked = false;
                if pasted.author.is_empty() {
                    author.clone_into(&mut pasted.author);
                }
                pasted
            })
            .collect()
    }

    /// As JSON, the app's own clipboard format.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "format": FORMAT,
            "annotations": self.annotations.iter().map(Annotation::to_json).collect::<Vec<_>>(),
        })
    }

    /// Read from [`Self::to_json`]'s form; `None` for anything else.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        if value.get("format")?.as_str()? != FORMAT {
            return None;
        }
        let annotations = value
            .get("annotations")?
            .as_array()?
            .iter()
            .filter_map(Annotation::from_json)
            .collect();
        Some(Self { annotations })
    }
}

/// The name and version of the clipboard format.
const FORMAT: &str = "excellentpdf.annotations/1";

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Markup, Path, Style};

    use super::*;

    fn point(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> PageRect {
        PageRect { x0, y0, x1, y1 }
    }

    fn square() -> Annotation {
        let mut square = Annotation::new(
            2,
            Shape::Boxed(Boxed::Square, rect(100.0, 100.0, 200.0, 150.0)),
            Style {
                color: Some([0.1, 0.2, 0.9]),
                opacity: 0.6,
                width: 2.0,
            },
            "Ada",
        );
        square.contents = "look here".to_owned();
        square
    }

    #[test]
    fn a_move_a_corner_and_a_vertex_drag_reshape_the_annotation() {
        let a = square();
        let moved =
            dragged(&a, Grip::Move, point(150.0, 120.0), point(160.0, 100.0)).expect("moves");
        assert_eq!(
            moved.shape,
            Shape::Boxed(Boxed::Square, rect(110.0, 80.0, 210.0, 130.0))
        );
        // The bottom-right corner, pulled out: the top left stays.
        let grown = dragged(
            &a,
            Grip::Corner(2),
            point(200.0, 150.0),
            point(260.0, 190.0),
        )
        .expect("resizes");
        assert_eq!(
            grown.shape,
            Shape::Boxed(Boxed::Square, rect(100.0, 100.0, 260.0, 190.0))
        );
        // Past the kept corner, it flips; never thinner than the least.
        let flipped =
            dragged(&a, Grip::Corner(2), point(200.0, 150.0), point(100.0, 60.0)).expect("flips");
        assert_eq!(
            flipped.shape,
            Shape::Boxed(Boxed::Square, rect(100.0, 60.0, 104.0, 100.0))
        );
        let line = Annotation::new(
            0,
            Shape::Line(point(0.0, 0.0), point(10.0, 0.0)),
            Style::default(),
            "Ada",
        );
        let bent =
            dragged(&line, Grip::Vertex(1), point(10.0, 0.0), point(10.0, 10.0)).expect("bends");
        assert_eq!(bent.shape, Shape::Line(point(0.0, 0.0), point(10.0, 10.0)));
        assert!(dragged(&line, Grip::Vertex(2), point(0.0, 0.0), point(1.0, 1.0)).is_none());
        assert!(
            dragged(&line, Grip::Corner(0), point(0.0, 0.0), point(1.0, 1.0)).is_none(),
            "a line has no corners"
        );
    }

    #[test]
    fn ink_and_callouts_scale_with_their_box() {
        let ink = Annotation::new(
            0,
            Shape::Ink(vec![vec![point(0.0, 0.0), point(10.0, 20.0)]]),
            Style::default(),
            "Ada",
        );
        let big =
            dragged(&ink, Grip::Corner(2), point(10.0, 20.0), point(20.0, 40.0)).expect("scales");
        assert_eq!(
            big.shape,
            Shape::Ink(vec![vec![point(0.0, 0.0), point(20.0, 40.0)]])
        );
        let callout = Annotation::new(
            0,
            Shape::Callout(rect(10.0, 10.0, 30.0, 20.0), point(0.0, 0.0)),
            Style::default(),
            "Ada",
        );
        let moved = moved(&callout, 5.0, 5.0).expect("moves");
        assert_eq!(
            moved.shape,
            Shape::Callout(rect(15.0, 15.0, 35.0, 25.0), point(5.0, 5.0))
        );
    }

    #[test]
    fn text_markup_and_locked_annotations_keep_still() {
        let highlight = Annotation::new(
            0,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        assert!(grips(&highlight).is_empty());
        assert!(moved(&highlight, 1.0, 1.0).is_none());
        let mut locked = square();
        locked.locked = true;
        assert!(grips(&locked).is_empty());
        assert!(moved(&locked, 1.0, 0.0).is_none());
        assert!(
            delete(&[locked.clone(), square()]).len() == 1,
            "the locked one stays"
        );
        let mut after = locked.clone();
        after.style.opacity = 1.0;
        assert!(change(&locked, after).is_none());
        let note = Annotation::new(
            0,
            Shape::Boxed(Boxed::Note, rect(0.0, 0.0, 20.0, 20.0)),
            Style::default(),
            "Ada",
        );
        assert!(
            grips(&note).is_empty(),
            "a sticky note moves but keeps its size"
        );
        assert!(moved(&note, 3.0, 4.0).is_some());
    }

    #[test]
    fn a_marquee_takes_what_lies_wholly_inside_it_and_ctrl_click_toggles() {
        let a = square();
        let polygon = Annotation::new(
            2,
            Shape::Path(
                Path::Polygon,
                vec![
                    point(300.0, 300.0),
                    point(320.0, 300.0),
                    point(310.0, 330.0),
                ],
            ),
            Style::default(),
            "Ada",
        );
        let both = [a.clone(), polygon.clone()];
        let taken = in_marquee(&both, rect(400.0, 400.0, 50.0, 50.0));
        assert_eq!(taken.len(), 2, "either way round");
        assert_eq!(in_marquee(&both, rect(50.0, 50.0, 250.0, 250.0)).len(), 1);
        let first = Target {
            page: 2,
            id: a.id.clone(),
        };
        let second = Target {
            page: 2,
            id: polygon.id.clone(),
        };
        let selection = toggled(std::slice::from_ref(&first), second.clone());
        assert_eq!(selection, vec![first.clone(), second.clone()]);
        assert_eq!(toggled(&selection, first.clone()), vec![second]);
        let elsewhere = Target {
            page: 5,
            id: "x".to_owned(),
        };
        assert_eq!(toggled(&selection, elsewhere.clone()), vec![elsewhere]);
    }

    #[test]
    fn the_canvas_s_drags_and_marquees_parse_and_a_move_takes_the_selection_along() {
        let data =
            json!({"page": 2, "id": "x", "grip": "move", "index": 0, "from": [0, 0], "to": [5, 5]});
        let drag = Drag::parse(&data).expect("parses");
        assert_eq!((drag.page, drag.grip), (2, Grip::Move));
        let other = square();
        assert!(
            drag.applied(&other, true).is_some(),
            "selected, it moves along"
        );
        assert!(drag.applied(&other, false).is_none());
        let bad = json!({"page": 2, "id": "x", "grip": "corner", "index": 9, "from": [0, 0], "to": [0, 0]});
        assert!(Drag::parse(&bad).is_none());
        let marquee = Marquee::parse(&json!({"page": 2, "rect": [50, 50, 250, 250], "add": true}))
            .expect("parses");
        assert!(marquee.add);
        assert_eq!(marquee.takes(std::slice::from_ref(&other)).len(), 1);
        assert!(Marquee::parse(&json!({"page": 1, "rect": [5, 6]})).is_none());
        let ink = Annotation::new(
            2,
            Shape::Ink(vec![vec![point(0.0, 0.0), point(10.0, 10.0)]]),
            Style::default(),
            "Ada",
        );
        let both = [other, ink.clone()];
        assert_eq!(
            hit(&both, point(5.0, 5.0)).map(|a| &a.id),
            Some(&ink.id),
            "ink is hit by its bounds"
        );
        assert!(hit(&both, point(500.0, 500.0)).is_none());
    }

    #[test]
    fn pasting_into_another_document_keeps_type_style_and_contents() {
        let copied = square();
        let clip = Clip::from_json(&Clip::of(vec![copied.clone()]).to_json()).expect("round trip");
        assert_eq!(clip.text(), "look here");
        // Onto another page (as into another document): where it was, with a new id.
        let pasted = clip.paste(0, PASTE_OFFSET, "Grace");
        assert_eq!(pasted.len(), 1);
        let p = &pasted[0];
        assert_ne!(p.id, copied.id);
        assert_eq!(p.page, 0);
        assert_eq!(p.shape, copied.shape);
        assert_eq!(p.style, copied.style);
        assert_eq!(p.contents, copied.contents);
        assert_eq!(p.author, "Ada", "who made it stays");
        // Onto its own page: offset, so it shows as a copy.
        let again = clip.paste(2, PASTE_OFFSET, "Grace");
        assert_eq!(
            again[0].shape,
            Shape::Boxed(Boxed::Square, rect(112.0, 112.0, 212.0, 162.0))
        );
        assert!(Clip::from_json(&json!({"format": "other"})).is_none());
        let highlight = Annotation::new(
            2,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style::default(),
            "Ada",
        );
        assert!(
            Clip::of(vec![highlight])
                .paste(3, PASTE_OFFSET, "Ada")
                .is_empty(),
            "markup stays with its text"
        );
    }
}
