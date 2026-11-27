//! An eraser's pass over a page's ink (H.5): the page canvas sends where the eraser went (its `erase` event), and
//! the ink annotations it crossed are deleted, or written again with what is left of their strokes, all in one
//! operation, so one undo brings everything back. Locked annotations (A.12) are left as they are.

use ecclesia_core::annot::{Annotation, Shape};
use ecclesia_core::edit::Change;
use ecclesia_ink::erase::{Cut, Eraser, Pass, Point, Target};
use ecclesia_mupdf::PagePoint;
use serde_json::Value;

use super::inspector::allows;
use super::markup::floats;

/// Where an eraser went on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Erased {
    /// The page, counting from zero.
    pub page: usize,
    /// Which eraser.
    pub eraser: Eraser,
    /// Its path, in page points, in the order drawn.
    pub path: Vec<PagePoint>,
    /// Its radius, in page points.
    pub radius: f32,
}

impl Erased {
    /// The pass in `value`, the event's data: `{page, eraser, radius, points: [[x, y]]}`, `eraser` an
    /// [`Eraser::name`]. `None` when a field is missing or wrong, or there are no points.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
        let eraser = Eraser::from_name(value.get("eraser")?.as_str()?)?;
        let radius = value.get("radius")?.as_f64()? as f32;
        let path = value
            .get("points")?
            .as_array()?
            .iter()
            .map(|point| match floats(point)?[..] {
                [x, y] => Some(PagePoint { x, y }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        (radius > 0.0 && !path.is_empty()).then_some(Self {
            page,
            eraser,
            path,
            radius,
        })
    }

    /// The changes this pass makes to `annotations`, the page's: each ink annotation it crosses deleted, or written
    /// again with the strokes it has left. Empty when it crossed none, or only locked ones.
    #[must_use]
    pub fn changes(&self, annotations: &[Annotation]) -> Vec<Change> {
        let path: Vec<Point> = self.path.iter().map(|p| point(*p)).collect();
        let pass = Pass::new(self.eraser, &path, self.radius);
        annotations
            .iter()
            .filter(|annotation| annotation.page == self.page)
            .filter_map(|annotation| {
                let change = erase(&pass, annotation)?;
                allows(annotation, &change).then_some(change)
            })
            .collect()
    }
}

/// What `pass` does to `annotation`: `None` when it isn't ink or isn't touched.
fn erase(pass: &Pass, annotation: &Annotation) -> Option<Change> {
    let Shape::Ink(strokes) = &annotation.shape else {
        return None;
    };
    let mut left: Vec<Vec<PagePoint>> = Vec::with_capacity(strokes.len());
    // Each stroke left keeps its widths (H.9); a cut one's pieces are drawn at the annotation's one width.
    let mut widths: Vec<Vec<f32>> = Vec::with_capacity(strokes.len());
    let mut touched = false;
    for (at, stroke) in strokes.iter().enumerate() {
        let path: Vec<Point> = stroke.iter().map(|p| point(*p)).collect();
        let target = Target {
            path: &path,
            width: annotation.style.width,
            highlighter: annotation.multiply,
        };
        match pass.cut(target) {
            Cut::Kept => {
                left.push(stroke.clone());
                widths.push(annotation.widths.get(at).cloned().unwrap_or_default());
            }
            Cut::Removed => touched = true,
            Cut::Split(pieces) => {
                touched = true;
                for piece in pieces {
                    left.push(
                        piece
                            .into_iter()
                            .map(|p| PagePoint { x: p.x, y: p.y })
                            .collect(),
                    );
                    widths.push(Vec::new());
                }
            }
        }
    }
    if !touched {
        return None;
    }
    if left.is_empty() {
        return Some(Change::DeleteAnnotation {
            page: annotation.page,
            id: annotation.id.clone(),
        });
    }
    let mut after = annotation.clone();
    after.shape = Shape::Ink(left);
    after.widths = widths;
    Some(Change::UpdateAnnotation(after))
}

fn point(p: PagePoint) -> Point {
    Point { x: p.x, y: p.y }
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::Style;
    use serde_json::json;

    use super::*;

    fn ink(id: &str, strokes: Vec<Vec<(f32, f32)>>) -> Annotation {
        let strokes = strokes
            .into_iter()
            .map(|s| s.into_iter().map(|(x, y)| PagePoint { x, y }).collect())
            .collect();
        let mut annotation = Annotation::new(
            0,
            Shape::Ink(strokes),
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

    fn pass(eraser: &str, points: serde_json::Value) -> Erased {
        Erased::parse(&json!({"page": 0, "eraser": eraser, "radius": 4.0, "points": points}))
            .expect("parses")
    }

    #[test]
    fn area_erasing_through_a_stroke_leaves_two_strokes_in_one_change() {
        let line = ink("a", vec![vec![(0.0, 0.0), (100.0, 0.0)]]);
        let changes = pass("area", json!([[50, -20], [50, 20]])).changes(&[line]);
        let [Change::UpdateAnnotation(after)] = &changes[..] else {
            panic!("one update: {changes:?}");
        };
        let Shape::Ink(strokes) = &after.shape else {
            panic!("still ink");
        };
        assert_eq!(strokes.len(), 2, "{strokes:?}");
        assert_eq!(
            after.id, "a",
            "the same annotation, so undo restores it whole"
        );
    }

    #[test]
    fn a_stroke_eraser_deletes_and_leaves_other_annotations() {
        let crossed = ink("a", vec![vec![(0.0, 0.0), (100.0, 0.0)]]);
        let apart = ink("b", vec![vec![(0.0, 50.0), (100.0, 50.0)]]);
        let changes = pass("stroke", json!([[50, -5], [50, 5]])).changes(&[crossed, apart]);
        assert_eq!(
            changes,
            vec![Change::DeleteAnnotation {
                page: 0,
                id: "a".into()
            }]
        );
    }

    #[test]
    fn one_stroke_of_two_goes_and_the_annotation_stays() {
        let two = ink(
            "a",
            vec![
                vec![(0.0, 0.0), (100.0, 0.0)],
                vec![(0.0, 50.0), (100.0, 50.0)],
            ],
        );
        let changes = pass("stroke", json!([[50, -5], [50, 5]])).changes(&[two]);
        let [Change::UpdateAnnotation(after)] = &changes[..] else {
            panic!("one update: {changes:?}");
        };
        assert_eq!(
            after.shape,
            Shape::Ink(vec![vec![
                PagePoint { x: 0.0, y: 50.0 },
                PagePoint { x: 100.0, y: 50.0 }
            ]])
        );
    }

    #[test]
    fn locked_and_highlighter_rules_hold() {
        let mut locked = ink("a", vec![vec![(0.0, 0.0), (100.0, 0.0)]]);
        locked.locked = true;
        assert!(
            pass("stroke", json!([[50, 0]]))
                .changes(&[locked])
                .is_empty()
        );
        let mut highlight = ink("h", vec![vec![(0.0, 0.0), (100.0, 0.0)]]);
        highlight.multiply = true;
        let pen = ink("p", vec![vec![(0.0, 0.0), (100.0, 0.0)]]);
        let changes = pass("highlighter", json!([[50, 0]])).changes(&[highlight, pen]);
        assert_eq!(
            changes,
            vec![Change::DeleteAnnotation {
                page: 0,
                id: "h".into()
            }]
        );
    }

    #[test]
    fn a_broken_pass_is_refused() {
        let parse = |value| Erased::parse(&value);
        assert_eq!(
            parse(json!({"page": 0, "eraser": "area", "radius": 4, "points": []})),
            None
        );
        assert_eq!(
            parse(json!({"page": 0, "eraser": "sponge", "radius": 4, "points": [[1, 2]]})),
            None
        );
        assert_eq!(
            parse(json!({"page": 0, "eraser": "area", "radius": 0, "points": [[1, 2]]})),
            None
        );
        assert_eq!(
            parse(json!({"page": 0, "eraser": "area", "radius": 4, "points": [[1]]})),
            None
        );
    }
}
