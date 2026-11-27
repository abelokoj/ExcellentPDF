//! The lasso's selection (H.6): a freeform loop the page canvas reports (`annotLasso`) selects the annotations it
//! encloses, as Enhanced does: an ink drawing when every point of it is inside, anything else when its four corners
//! are. Then the selection is worked on whole: resized from its own box ([`GROUP`]), turned about its centre, made
//! thinner or thicker, recoloured, or duplicated (A.13's clipboard). Each is one step of undo.

use ecclesia_core::annot::{Annotation, Shape};
use ecclesia_core::edit::Change;
use ecclesia_ink::erase::Point;
use ecclesia_ink::lasso::{Lasso, rotate};
use ecclesia_mupdf::{PagePoint, PageRect};
use serde_json::Value;

use super::edit::{self, bounds, can_move, corners, map_shape};
use super::markup::floats;
use super::notes::Target;

/// The id the app gives the box around a selection of several annotations, offered as one more item with corner
/// grips: dragging one of its corners scales them all; dragging inside it moves them all.
pub const GROUP: &str = "*selection*";

/// The thinnest and thickest width the width commands give, in points (Enhanced's limits).
const WIDTHS: (f32, f32) = (0.1, 72.0);

/// What the thinner and thicker commands multiply widths by (Enhanced's steps).
pub const THINNER: f32 = 0.8;
/// See [`THINNER`].
pub const THICKER: f32 = 1.25;

/// A loop the canvas reports (`annotLasso`: `{page, path: [[x, y]], add}`, page points; `add` when Ctrl was down).
#[derive(Debug, Clone, PartialEq)]
pub struct Looped {
    /// The page it was drawn on.
    pub page: usize,
    /// The loop.
    pub path: Vec<PagePoint>,
    /// Whether it adds to the selection.
    pub add: bool,
}

impl Looped {
    /// The loop `data` describes; `None` when it doesn't parse or has fewer than three points.
    #[must_use]
    pub fn parse(data: &Value) -> Option<Self> {
        let path = data
            .get("path")?
            .as_array()?
            .iter()
            .map(|point| match floats(point)?[..] {
                [x, y] => Some(PagePoint { x, y }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        (path.len() >= 3).then_some(())?;
        Some(Self {
            page: usize::try_from(data.get("page")?.as_u64()?).ok()?,
            path,
            add: data.get("add").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// The annotations of `annotations` on its page it encloses.
    #[must_use]
    pub fn takes(&self, annotations: &[Annotation]) -> Vec<Target> {
        let lasso = Lasso::new(self.path.iter().map(|p| point(*p)).collect());
        annotations
            .iter()
            .filter(|a| a.page == self.page && encloses(&lasso, a))
            .map(|a| Target {
                page: a.page,
                id: a.id.clone(),
            })
            .collect()
    }
}

/// Whether `lasso` encloses `annotation`: every point of an ink drawing, the four corners of anything else.
fn encloses(lasso: &Lasso, annotation: &Annotation) -> bool {
    match &annotation.shape {
        Shape::Ink(strokes) => lasso.encloses(strokes.iter().flatten().map(|p| point(*p))),
        _ => bounds(annotation)
            .is_some_and(|rect| lasso.encloses(corners(rect).into_iter().map(point))),
    }
}

/// The box around `annotations`; `None` for none with points.
#[must_use]
pub fn group_bounds(annotations: &[Annotation]) -> Option<PageRect> {
    annotations
        .iter()
        .filter_map(bounds)
        .reduce(|a, b| PageRect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
}

/// The changes that scale `annotations` from their box to the box with corner `index` (clockwise from the top
/// left) dragged to `to`; the locked ones and text markup stay.
#[must_use]
pub fn group_resized(annotations: &[Annotation], index: usize, to: PagePoint) -> Vec<Change> {
    let Some(old) = group_bounds(annotations) else {
        return Vec::new();
    };
    let new = edit::resized(old, index, to);
    changes(annotations, |a| {
        can_move(a).then(|| map_shape(&a.shape, |p| edit::scale(p, old, new)))
    })
}

/// Whether `annotation` can be turned: movable, and drawn through points (ink, a line, a polygon or polyline). A
/// box stays upright in PDF.
#[must_use]
pub fn can_rotate(annotation: &Annotation) -> bool {
    can_move(annotation)
        && matches!(
            annotation.shape,
            Shape::Ink(_) | Shape::Line(..) | Shape::Path(..)
        )
}

/// The changes that turn `annotations` `degrees` clockwise about the centre of their box; none unless each can
/// turn, so a selection never comes out half turned.
#[must_use]
pub fn rotated(annotations: &[Annotation], degrees: f32) -> Vec<Change> {
    if annotations.is_empty() || !annotations.iter().all(can_rotate) {
        return Vec::new();
    }
    let Some(rect) = group_bounds(annotations) else {
        return Vec::new();
    };
    let centre = Point {
        x: (rect.x0 + rect.x1) / 2.0,
        y: (rect.y0 + rect.y1) / 2.0,
    };
    changes(annotations, |a| {
        Some(map_shape(&a.shape, |p| {
            let turned = rotate(point(p), centre, degrees);
            PagePoint {
                x: turned.x,
                y: turned.y,
            }
        }))
    })
}

/// The changes that multiply the width of each of `annotations` that has one by `factor`, within Enhanced's limits.
#[must_use]
pub fn width_scaled(annotations: &[Annotation], factor: f32) -> Vec<Change> {
    annotations
        .iter()
        .filter(|a| a.shape.has_width())
        .filter_map(|a| {
            let mut after = a.clone();
            after.style.width = (a.style.width * factor).clamp(WIDTHS.0, WIDTHS.1);
            edit::change(a, after)
        })
        .collect()
}

/// The changes that give each of `annotations` the colour `color` (RGB from 0 to 1).
#[must_use]
pub fn recolored(annotations: &[Annotation], color: [f32; 3]) -> Vec<Change> {
    annotations
        .iter()
        .filter_map(|a| {
            let mut after = a.clone();
            after.style.color = Some(color);
            edit::change(a, after)
        })
        .collect()
}

/// The changes that give each of `annotations` the shape `reshape` makes of it, where it makes one.
fn changes(
    annotations: &[Annotation],
    reshape: impl Fn(&Annotation) -> Option<Shape>,
) -> Vec<Change> {
    annotations
        .iter()
        .filter_map(|a| {
            let mut after = a.clone();
            after.shape = reshape(a)?;
            edit::change(a, after)
        })
        .collect()
}

fn point(p: PagePoint) -> Point {
    Point { x: p.x, y: p.y }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::{Boxed, Style};
    use serde_json::json;

    use super::*;

    fn make(id: &str, shape: Shape) -> Annotation {
        let mut annotation = Annotation::new(
            0,
            shape,
            Style {
                color: Some([0.0, 0.0, 1.0]),
                opacity: 1.0,
                width: 2.0,
            },
            "Ada",
        );
        annotation.id = id.to_owned();
        annotation
    }

    fn pt(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn ink(id: &str, points: &[(f32, f32)]) -> Annotation {
        make(
            id,
            Shape::Ink(vec![points.iter().map(|&(x, y)| pt(x, y)).collect()]),
        )
    }

    fn updated(changes: &[Change]) -> Vec<&Annotation> {
        changes
            .iter()
            .map(|change| match change {
                Change::UpdateAnnotation(a) => a,
                other => panic!("an update: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_loop_takes_ink_wholly_inside_and_boxes_by_their_corners() {
        let inside = ink("in", &[(10.0, 10.0), (20.0, 20.0)]);
        let half = ink("half", &[(10.0, 10.0), (200.0, 20.0)]);
        let square = make(
            "box",
            Shape::Boxed(
                Boxed::Square,
                PageRect {
                    x0: 30.0,
                    y0: 30.0,
                    x1: 40.0,
                    y1: 40.0,
                },
            ),
        );
        let looped = Looped::parse(&json!({
            "page": 0, "path": [[0, 0], [100, 0], [100, 100], [0, 100]], "add": false,
        }))
        .expect("parses");
        let taken: Vec<String> = looped
            .takes(&[inside, half, square])
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(taken, ["in", "box"]);
    }

    #[test]
    fn a_loop_needs_three_points() {
        assert_eq!(
            Looped::parse(&json!({"page": 0, "path": [[0, 0], [5, 5]]})),
            None
        );
    }

    #[test]
    fn the_selection_scales_from_its_own_box() {
        let a = ink("a", &[(0.0, 0.0), (10.0, 10.0)]);
        let b = ink("b", &[(10.0, 10.0), (20.0, 20.0)]);
        // The bottom-right corner (index 2) from (20, 20) to (40, 40): everything doubles from the top left.
        let changes = group_resized(&[a, b], 2, pt(40.0, 40.0));
        let after = updated(&changes);
        assert_eq!(
            after[1].shape,
            Shape::Ink(vec![vec![pt(20.0, 20.0), pt(40.0, 40.0)]])
        );
    }

    #[test]
    fn turning_goes_about_the_centre_and_needs_every_one_to_turn() {
        let line = make("l", Shape::Line(pt(0.0, 0.0), pt(10.0, 0.0)));
        let changes = rotated(std::slice::from_ref(&line), 90.0);
        let Shape::Line(from, to) = updated(&changes)[0].shape else {
            panic!("a line");
        };
        assert!(
            (from.x - 5.0).abs() < 1e-4 && (from.y + 5.0).abs() < 1e-4,
            "{from:?}"
        );
        assert!(
            (to.x - 5.0).abs() < 1e-4 && (to.y - 5.0).abs() < 1e-4,
            "{to:?}"
        );
        let square = make(
            "s",
            Shape::Boxed(
                Boxed::Square,
                PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 5.0,
                    y1: 5.0,
                },
            ),
        );
        assert!(
            rotated(&[line, square], 90.0).is_empty(),
            "a box can't turn"
        );
    }

    #[test]
    fn widths_step_within_their_limits_and_colours_change() {
        let mut thin = ink("t", &[(0.0, 0.0), (1.0, 1.0)]);
        thin.style.width = 0.11;
        let note = make(
            "n",
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 20.0,
                    y1: 20.0,
                },
            ),
        );
        let changes = width_scaled(&[thin, note.clone()], THINNER);
        let after = updated(&changes);
        assert_eq!(after.len(), 1, "a note has no width");
        assert!((after[0].style.width - WIDTHS.0).abs() < 1e-6);
        let recolor = recolored(&[note], [1.0, 0.0, 0.0]);
        assert_eq!(updated(&recolor)[0].style.color, Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn locked_ones_stay() {
        let mut locked = ink("a", &[(0.0, 0.0), (10.0, 10.0)]);
        locked.locked = true;
        assert!(rotated(std::slice::from_ref(&locked), 90.0).is_empty());
        assert!(group_resized(&[locked], 2, pt(40.0, 40.0)).is_empty());
    }
}
