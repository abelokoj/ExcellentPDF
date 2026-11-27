//! Convert recognized handwritten shapes to vector PDF annotations (H.26, plan §27.7): runs H.10's shape recognizer
//! over selected ink strokes and converts lines, arrows, rectangles, circles/ellipses, and polygons into standard PDF
//! `Line`, `Square`, `Circle`, and `Polygon` annotations (A.8–A.10) that are immediately editable with A.13 grips and
//! handles.

use ecclesia_core::annot::{Annotation, Boxed, LineEnd, Path, Shape};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_ink::erase::Point;
use ecclesia_ink::shapes::{self, Kind as RecognizedKind, Shape as RecognizedShape};
use ecclesia_mupdf::{PagePoint, PageRect};

use crate::ink::layers::LayerStack;

/// Maximum tilt from horizontal/vertical (in degrees) for a recognized rectangle to become an axis-aligned PDF
/// `Square` annotation; steeper rotated rectangles become `Polygon` annotations so their orientation is preserved.
const AXIS_ALIGN_TOLERANCE_DEG: f32 = 12.0;

/// Result of converting a selection of ink annotations into vector PDF shape annotations.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorizeResult {
    /// Single undoable operation deleting/updating the source ink and creating the vector annotations.
    pub operation: Operation,
    /// Newly created vector shape annotations, in order.
    pub created: Vec<Annotation>,
    /// Number of ink strokes that did not match any shape (kept as ink).
    pub unrecognized_strokes: usize,
}

/// Attempts to recognize a single stroke and convert it into a vector PDF annotation (`Line`, `Square`, `Circle`, or
/// `Polygon`) preserving `source`'s page, style, author, layer, category, and tags.
///
/// Returns `None` if the stroke is not recognized as a geometric shape (for example handwriting or a scribble).
#[must_use]
pub fn convert_stroke(source: &Annotation, stroke: &[PagePoint]) -> Option<Annotation> {
    let pts: Vec<Point> = stroke.iter().map(|p| Point { x: p.x, y: p.y }).collect();
    let recognized = shapes::recognize(&pts)?;
    let kind = recognized.kind();

    let (shape, ends) = match recognized {
        RecognizedShape::Line(a, b) => (
            Shape::Line(to_page_point(a), to_page_point(b)),
            [LineEnd::None, LineEnd::None],
        ),
        RecognizedShape::Arrow { tail, tip, .. } => (
            Shape::Line(to_page_point(tail), to_page_point(tip)),
            [LineEnd::None, LineEnd::OpenArrow],
        ),
        RecognizedShape::Polygon(corners) => {
            if kind == RecognizedKind::Rectangle && is_axis_aligned(&corners) {
                let rect = bounding_rect(&corners)?;
                (Shape::Boxed(Boxed::Square, rect), [LineEnd::None; 2])
            } else {
                let verts: Vec<PagePoint> = corners.into_iter().map(to_page_point).collect();
                (Shape::Path(Path::Polygon, verts), [LineEnd::None; 2])
            }
        }
        RecognizedShape::Ellipse {
            centre,
            rx,
            ry,
            angle,
        } => {
            // Compute the tight axis-aligned bounding box of the ellipse so PDF's Circle annotation covers it cleanly.
            let (sin, cos) = angle.sin_cos();
            let half_w = (rx * cos).hypot(ry * sin).max(1.0);
            let half_h = (rx * sin).hypot(ry * cos).max(1.0);
            let rect = PageRect {
                x0: centre.x - half_w,
                y0: centre.y - half_h,
                x1: centre.x + half_w,
                y1: centre.y + half_h,
            };
            (Shape::Boxed(Boxed::Circle, rect), [LineEnd::None; 2])
        }
    };

    let mut vector = Annotation::new(source.page, shape, source.style, &source.author);
    vector.ends = ends;
    vector.layer.clone_from(&source.layer);
    vector.category.clone_from(&source.category);
    vector.tags.clone_from(&source.tags);
    Some(vector)
}

/// Converts all recognized shape strokes across `annotations` into standard vector annotations inside one undoable
/// [`Operation`].
///
/// Unrecognized strokes stay as ink; locked annotations and annotations on locked layers (when `layers` is given) are
/// skipped untouched. Returns `None` when no stroke in `annotations` converts.
#[must_use]
pub fn convert_operation(
    annotations: &[Annotation],
    layers: Option<&LayerStack>,
) -> Option<VectorizeResult> {
    let mut changes: Vec<Change> = Vec::new();
    let mut created: Vec<Annotation> = Vec::new();
    let mut unrecognized_strokes = 0usize;

    for annotation in annotations {
        let Shape::Ink(strokes) = &annotation.shape else {
            continue;
        };
        if annotation.locked || layers.is_some_and(|stack| stack.is_locked(annotation)) {
            continue;
        }

        let mut remaining_strokes: Vec<Vec<PagePoint>> = Vec::new();
        let mut remaining_widths: Vec<Vec<f32>> = Vec::new();
        let mut made_for_annot: Vec<Annotation> = Vec::new();

        for (idx, stroke) in strokes.iter().enumerate() {
            if let Some(mut vector) = convert_stroke(annotation, stroke) {
                if strokes.len() == 1 && !annotation.contents.is_empty() {
                    vector.contents.clone_from(&annotation.contents);
                }
                made_for_annot.push(vector);
            } else {
                unrecognized_strokes += 1;
                remaining_strokes.push(stroke.clone());
                if let Some(w) = annotation.widths.get(idx) {
                    remaining_widths.push(w.clone());
                }
            }
        }

        if made_for_annot.is_empty() {
            continue;
        }

        if remaining_strokes.is_empty() {
            changes.push(Change::DeleteAnnotation {
                page: annotation.page,
                id: annotation.id.clone(),
            });
        } else {
            let mut kept = annotation.clone();
            kept.shape = Shape::Ink(remaining_strokes);
            kept.widths = remaining_widths;
            changes.push(Change::UpdateAnnotation(kept));
        }

        for vector in made_for_annot {
            changes.push(Change::CreateAnnotation(vector.clone()));
            created.push(vector);
        }
    }

    if created.is_empty() {
        return None;
    }

    let label = if created.len() == 1 {
        "Convert shape to vector".to_owned()
    } else {
        format!("Convert {} shapes to vectors", created.len())
    };
    Some(VectorizeResult {
        operation: Operation::new(label, changes),
        created,
        unrecognized_strokes,
    })
}

fn to_page_point(p: Point) -> PagePoint {
    PagePoint { x: p.x, y: p.y }
}

fn bounding_rect(corners: &[Point]) -> Option<PageRect> {
    let first = corners.first()?;
    Some(corners.iter().fold(
        PageRect {
            x0: first.x,
            y0: first.y,
            x1: first.x,
            y1: first.y,
        },
        |r, p| PageRect {
            x0: r.x0.min(p.x),
            y0: r.y0.min(p.y),
            x1: r.x1.max(p.x),
            y1: r.y1.max(p.y),
        },
    ))
}

fn is_axis_aligned(corners: &[Point]) -> bool {
    if corners.len() != 4 {
        return false;
    }
    let dx = corners[1].x - corners[0].x;
    let dy = corners[1].y - corners[0].y;
    let angle_deg = dy.atan2(dx).to_degrees().rem_euclid(90.0);
    angle_deg <= AXIS_ALIGN_TOLERANCE_DEG || (90.0 - angle_deg) <= AXIS_ALIGN_TOLERANCE_DEG
}

#[cfg(test)]
mod tests {
    use ecclesia_core::annot::Style;

    use crate::annotate::edit::{Grip, dragged, grips};
    use crate::annotate::shapes::{Kind as AnnotShapeKind, is_arrow, kind_of};
    use crate::ink::layers::LockState;

    use super::*;

    fn pt(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn ink_annot(id: &str, strokes: Vec<Vec<PagePoint>>) -> Annotation {
        let mut a = Annotation::new(
            0,
            Shape::Ink(strokes),
            Style {
                color: Some([0.1, 0.3, 0.8]),
                opacity: 0.9,
                width: 2.5,
            },
            "Ada",
        );
        a.id = id.to_owned();
        a.layer = Some("Ink".into());
        a
    }

    fn sampled_segment(a: PagePoint, b: PagePoint, steps: usize) -> Vec<PagePoint> {
        (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
            })
            .collect()
    }

    fn sampled_polygon(verts: &[PagePoint]) -> Vec<PagePoint> {
        let mut out = Vec::new();
        for i in 0..verts.len() {
            let a = verts[i];
            let b = verts[(i + 1) % verts.len()];
            let seg = sampled_segment(a, b, 10);
            if !out.is_empty() {
                out.extend_from_slice(&seg[1..]);
            } else {
                out.extend(seg);
            }
        }
        out
    }

    fn sampled_circle(cx: f32, cy: f32, r: f32) -> Vec<PagePoint> {
        (0..=64)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 64.0;
                pt(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect()
    }

    #[test]
    fn converted_shapes_are_editable_with_a13_handles() {
        let line_ink = ink_annot(
            "line",
            vec![sampled_segment(pt(10.0, 10.0), pt(110.0, 10.0), 16)],
        );

        let mut arrow_pts = sampled_segment(pt(10.0, 40.0), pt(110.0, 40.0), 16);
        arrow_pts.extend(
            sampled_segment(pt(110.0, 40.0), pt(90.0, 30.0), 6)
                .into_iter()
                .skip(1),
        );
        arrow_pts.extend(
            sampled_segment(pt(90.0, 30.0), pt(110.0, 40.0), 6)
                .into_iter()
                .skip(1),
        );
        arrow_pts.extend(
            sampled_segment(pt(110.0, 40.0), pt(90.0, 50.0), 6)
                .into_iter()
                .skip(1),
        );
        let arrow_ink = ink_annot("arrow", vec![arrow_pts]);

        let rect_ink = ink_annot(
            "rect",
            vec![sampled_polygon(&[
                pt(10.0, 70.0),
                pt(110.0, 70.0),
                pt(110.0, 130.0),
                pt(10.0, 130.0),
            ])],
        );

        let circle_ink = ink_annot("circle", vec![sampled_circle(200.0, 100.0, 40.0)]);

        let triangle_ink = ink_annot(
            "tri",
            vec![sampled_polygon(&[
                pt(300.0, 60.0),
                pt(350.0, 140.0),
                pt(250.0, 140.0),
            ])],
        );

        let pentagon_verts: Vec<PagePoint> = (0..5)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / 5.0;
                pt(420.0 + 40.0 * a.cos(), 100.0 + 40.0 * a.sin())
            })
            .collect();
        let pentagon_ink = ink_annot("pent", vec![sampled_polygon(&pentagon_verts)]);

        // A zig-zag scribble that must NOT convert.
        let scribble: Vec<PagePoint> = (0..25)
            .map(|i| {
                pt(
                    500.0 + i as f32 * 4.0,
                    100.0 + if i % 2 == 0 { -25.0 } else { 25.0 },
                )
            })
            .collect();
        let scribble_ink = ink_annot("scribble", vec![scribble]);

        let selection = [
            line_ink,
            arrow_ink,
            rect_ink,
            circle_ink,
            triangle_ink,
            pentagon_ink,
            scribble_ink,
        ];

        let result = convert_operation(&selection, None).expect("converts shapes");
        assert_eq!(result.created.len(), 6);
        assert_eq!(result.unrecognized_strokes, 1);

        // 1. Line -> Line annotation with 2 Vertex grips, draggable via A.13.
        let line = &result.created[0];
        assert_eq!(kind_of(line), Some(AnnotShapeKind::Line));
        assert_eq!(grips(line).len(), 2);
        let moved_line =
            dragged(line, Grip::Vertex(1), pt(110.0, 10.0), pt(150.0, 25.0)).expect("drags line");
        assert_eq!(
            moved_line.shape,
            Shape::Line(pt(10.0, 10.0), pt(150.0, 25.0))
        );

        // 2. Arrow -> Line with OpenArrow ending, draggable via A.13.
        let arrow = &result.created[1];
        assert!(is_arrow(arrow));
        assert_eq!(grips(arrow).len(), 2);
        assert!(dragged(arrow, Grip::Vertex(0), pt(10.0, 40.0), pt(0.0, 40.0)).is_some());

        // 3. Rectangle -> Square annotation with 4 Corner grips, resizable via A.13.
        let square = &result.created[2];
        assert_eq!(kind_of(square), Some(AnnotShapeKind::Rectangle));
        let sq_grips = grips(square);
        assert_eq!(sq_grips.len(), 4);
        assert_eq!(sq_grips[2].0, Grip::Corner(2));
        let resized_sq = dragged(square, Grip::Corner(2), sq_grips[2].1, pt(160.0, 180.0))
            .expect("resizes square");
        let Shape::Boxed(Boxed::Square, rect) = resized_sq.shape else {
            panic!("expected square");
        };
        assert!((rect.x1 - 160.0).abs() < 0.1 && (rect.y1 - 180.0).abs() < 0.1);

        // 4. Circle -> Circle annotation with 4 Corner grips, resizable via A.13.
        let circle = &result.created[3];
        assert_eq!(kind_of(circle), Some(AnnotShapeKind::Ellipse));
        let c_grips = grips(circle);
        assert_eq!(c_grips.len(), 4);
        assert!(dragged(circle, Grip::Corner(2), c_grips[2].1, pt(280.0, 180.0)).is_some());

        // 5. Triangle -> Polygon with 3 Vertex grips, vertex draggable via A.13.
        let tri = &result.created[4];
        assert_eq!(kind_of(tri), Some(AnnotShapeKind::Polygon));
        let tri_grips = grips(tri);
        assert_eq!(tri_grips.len(), 3);
        let edited_tri =
            dragged(tri, Grip::Vertex(0), tri_grips[0].1, pt(310.0, 40.0)).expect("drags vertex");
        let Shape::Path(Path::Polygon, verts) = edited_tri.shape else {
            panic!("expected polygon");
        };
        assert_eq!(verts[0], pt(310.0, 40.0));

        // 6. Pentagon -> Polygon with 5 Vertex grips.
        let pent = &result.created[5];
        assert_eq!(kind_of(pent), Some(AnnotShapeKind::Polygon));
        assert_eq!(grips(pent).len(), 5);

        // Metadata preserved.
        assert_eq!(square.layer.as_deref(), Some("Ink"));
        assert_eq!(square.style.width, 2.5);
    }

    #[test]
    fn mixed_annotation_keeps_unrecognized_strokes_and_respects_layer_locks() {
        let line = sampled_segment(pt(0.0, 0.0), pt(80.0, 0.0), 12);
        let scribble: Vec<PagePoint> = (0..20)
            .map(|i| pt(i as f32 * 5.0, 50.0 + if i % 2 == 0 { -20.0 } else { 20.0 }))
            .collect();
        let mixed = ink_annot("mixed", vec![line.clone(), scribble]);

        let result =
            convert_operation(std::slice::from_ref(&mixed), None).expect("converts 1 of 2");
        assert_eq!(result.created.len(), 1);
        assert_eq!(result.unrecognized_strokes, 1);
        assert!(
            matches!(&result.operation.changes[0], Change::UpdateAnnotation(a) if matches!(&a.shape, Shape::Ink(s) if s.len() == 1)),
            "unrecognized stroke stays in updated ink annotation"
        );

        let mut stack = LayerStack::new();
        stack.set_lock("Ink", LockState::Locked).expect("locks Ink");
        assert!(
            convert_operation(&[mixed], Some(&stack)).is_none(),
            "locked layer is not converted"
        );
    }
}
