//! Shape recognition (H.10, plan §8.4): when the pen is held still at the end of a stroke, the stroke is read as the
//! shape it was meant to be, a line, an arrow, a triangle, a rectangle, a polygon, a circle or an ellipse, and
//! replaced by that shape drawn true, in the same pen. A stroke that fits none closely stays as it was drawn.
//!
//! Corners come from Douglas-Peucker simplification; an ellipse from the stroke's second moments (its centre and
//! axes), checked by how far each point lies from it.

use crate::erase::Point;

/// How long the pen must rest at the end of a stroke for it to be snapped, in milliseconds.
pub const HOLD_MS: f64 = 500.0;

/// A stroke whose ends are closer than this share of its length is closed (a loop).
const CLOSED: f32 = 0.18;
/// Douglas-Peucker's tolerance, as a share of the stroke's size (its box's diagonal).
const CORNER_TOLERANCE: f32 = 0.06;
/// The most a point may stray from a fitted shape, on average, as a share of the stroke's size.
const FIT: f32 = 0.06;
/// An ellipse whose axes differ by less than this share is a circle.
const ROUND: f32 = 0.12;
/// Points on a fitted circle or ellipse.
const ROUND_POINTS: usize = 64;
/// The most corners a recognized polygon has: past this the stroke is a drawing, not a shape.
const MAX_CORNERS: usize = 8;
/// How much longer than its shape's own outline a stroke may be: one that goes over its area again and again (a
/// scribble, shading) is no shape.
const RETRACE: f32 = 1.6;

/// A recognized shape, its points in page points.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A straight line from one end to the other.
    Line(Point, Point),
    /// A line from its tail to its tip, with a head at the tip.
    Arrow {
        /// Where it starts.
        tail: Point,
        /// Where it points.
        tip: Point,
        /// The head's two barbs' ends.
        barbs: [Point; 2],
    },
    /// A closed figure through its corners: three for a triangle, four for a rectangle, more for a polygon.
    Polygon(Vec<Point>),
    /// An ellipse (a circle when its radii are equal): its centre, its radii and its turn, in radians.
    Ellipse {
        /// Its centre.
        centre: Point,
        /// Its radius along its first axis.
        rx: f32,
        /// Its radius along its second axis.
        ry: f32,
        /// How far its first axis is turned from across the page, in radians.
        angle: f32,
    },
}

/// What a shape is, for tests and for saying what a stroke became.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A line.
    Line,
    /// An arrow.
    Arrow,
    /// Three corners.
    Triangle,
    /// Four corners at right angles.
    Rectangle,
    /// Four or more other corners.
    Polygon,
    /// Equal radii.
    Circle,
    /// Unequal radii.
    Ellipse,
}

impl Shape {
    /// What it is.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Line(..) => Kind::Line,
            Self::Arrow { .. } => Kind::Arrow,
            Self::Polygon(corners) if corners.len() == 3 => Kind::Triangle,
            Self::Polygon(corners) if corners.len() == 4 && is_rectangle(corners) => {
                Kind::Rectangle
            }
            Self::Polygon(_) => Kind::Polygon,
            Self::Ellipse { rx, ry, .. } if (rx - ry).abs() <= rx.max(*ry) * ROUND => Kind::Circle,
            Self::Ellipse { .. } => Kind::Ellipse,
        }
    }

    /// The path the pen draws it with: one polyline, an arrow's head retraced from its tip, a closed figure back to
    /// its first point.
    #[must_use]
    pub fn path(&self) -> Vec<Point> {
        match self {
            Self::Line(a, b) => vec![*a, *b],
            Self::Arrow { tail, tip, barbs } => vec![*tail, *tip, barbs[0], *tip, barbs[1]],
            Self::Polygon(corners) => corners.iter().chain(corners.first()).copied().collect(),
            Self::Ellipse {
                centre,
                rx,
                ry,
                angle,
            } => {
                let (sin, cos) = angle.sin_cos();
                (0..=ROUND_POINTS)
                    .map(|i| {
                        let t = std::f32::consts::TAU * i as f32 / ROUND_POINTS as f32;
                        let (x, y) = (rx * t.cos(), ry * t.sin());
                        Point {
                            x: centre.x + x * cos - y * sin,
                            y: centre.y + x * sin + y * cos,
                        }
                    })
                    .collect()
            }
        }
    }
}

/// The shape the stroke through `points` was meant to be, or `None` when it fits none closely.
#[must_use]
pub fn recognize(points: &[Point]) -> Option<Shape> {
    if points.len() < 3 {
        return None;
    }
    let size = diagonal(points);
    if size <= f32::EPSILON {
        return None;
    }
    let length = path_length(points);
    let ends = distance(points[0], points[points.len() - 1]);
    if ends < length * CLOSED {
        closed(points, size)
    } else {
        open(points, size)
    }
}

/// An open stroke: a line, or a line with a head at its end.
fn open(points: &[Point], size: f32) -> Option<Shape> {
    let (first, last) = (points[0], points[points.len() - 1]);
    if mean_distance(points, &[first, last], false) <= size * FIT / 2.0 {
        return Some(Shape::Line(first, last));
    }
    let corners = simplify(points, size * CORNER_TOLERANCE);
    // A shaft, then a head: the stroke turns back at the tip and the rest stays near it.
    if corners.len() >= 3 {
        let (tail, tip) = (corners[0], corners[1]);
        let shaft = distance(tail, tip);
        let head: Vec<Point> = corners[2..].to_vec();
        let near_tip = head.iter().all(|p| distance(*p, tip) <= shaft * 0.45);
        let on_shaft = mean_distance(&points[..=index_of(points, tip)], &[tail, tip], false)
            <= size * FIT / 2.0;
        if near_tip && on_shaft && shaft >= size * 0.6 {
            return Some(Shape::Arrow {
                tail,
                tip,
                barbs: barbs(tail, tip, shaft),
            });
        }
    }
    None
}

/// A closed stroke: a polygon through its corners, or an ellipse, whichever fits closer.
fn closed(points: &[Point], size: f32) -> Option<Shape> {
    let polygon = corners(points, size);
    let polygon_error = polygon
        .as_ref()
        .map(|c| mean_distance(points, c, true))
        .unwrap_or(f32::INFINITY);
    let ellipse = fit_ellipse(points);
    let ellipse_error = ellipse
        .as_ref()
        .map(|e| ellipse_distance(points, e))
        .unwrap_or(f32::INFINITY);
    let (shape, error) = match (polygon, ellipse) {
        // A polygon wins only clearly: a wobbly circle has corners too.
        (Some(p), Some(_))
            if polygon_error < ellipse_error * 0.8
                || p.len() == 3 && polygon_error < ellipse_error =>
        {
            (Shape::Polygon(p), polygon_error)
        }
        (_, Some(e)) => (e, ellipse_error),
        (Some(p), None) => (Shape::Polygon(p), polygon_error),
        (None, None) => return None,
    };
    if error > size * FIT || path_length(points) > path_length(&shape.path()) * RETRACE {
        return None;
    }
    Some(match shape {
        Shape::Polygon(c) if c.len() == 4 && is_rectangle(&c) => Shape::Polygon(squared(&c)),
        Shape::Ellipse {
            centre,
            rx,
            ry,
            angle,
        } if (rx - ry).abs() <= rx.max(ry) * ROUND => {
            let r = (rx + ry) / 2.0;
            Shape::Ellipse {
                centre,
                rx: r,
                ry: r,
                angle,
            }
        }
        other => other,
    })
}

/// A closed stroke's corners, three or more, the end that closes it merged with the start.
fn corners(points: &[Point], size: f32) -> Option<Vec<Point>> {
    let mut corners = simplify(points, size * CORNER_TOLERANCE);
    if corners.len() >= 2 && distance(corners[0], corners[corners.len() - 1]) < size * CLOSED {
        let last = corners.pop().expect("two or more");
        corners[0] = midpoint(corners[0], last);
    }
    // Where the stroke starts mid-side the start is no corner: a corner sits where the outline turns sharply.
    let corners = drop_flat(corners, 0.35);
    (3..=MAX_CORNERS)
        .contains(&corners.len())
        .then_some(corners)
}

/// `corners` without those where the outline barely turns (the turn's sine under `flat`).
fn drop_flat(corners: Vec<Point>, flat: f32) -> Vec<Point> {
    let n = corners.len();
    if n <= 3 {
        return corners;
    }
    let kept: Vec<Point> = (0..n)
        .filter(|&i| {
            let (a, b, c) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
            turn(a, b, c) >= flat
        })
        .map(|i| corners[i])
        .collect();
    if kept.len() >= 3 { kept } else { corners }
}

/// How sharply the outline turns at `b`, from 0 (straight on) to 1 (a right angle or sharper).
fn turn(a: Point, b: Point, c: Point) -> f32 {
    let (u, v) = ((b.x - a.x, b.y - a.y), (c.x - b.x, c.y - b.y));
    let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
    if lu <= f32::EPSILON || lv <= f32::EPSILON {
        return 0.0;
    }
    let cross = (u.0 * v.1 - u.1 * v.0) / (lu * lv);
    let dot = (u.0 * v.0 + u.1 * v.1) / (lu * lv);
    if dot < 0.0 { 1.0 } else { cross.abs() }
}

/// Whether four corners make a rectangle: each angle within 15 degrees of a right angle.
fn is_rectangle(c: &[Point]) -> bool {
    c.len() == 4
        && (0..4).all(|i| {
            let (a, b, d) = (c[(i + 3) % 4], c[i], c[(i + 1) % 4]);
            let (u, v) = ((a.x - b.x, a.y - b.y), (d.x - b.x, d.y - b.y));
            let cos = (u.0 * v.0 + u.1 * v.1) / (u.0.hypot(u.1) * v.0.hypot(v.1)).max(f32::EPSILON);
            cos.abs() < 0.26
        })
}

/// Four corners made into a true rectangle: the first side's direction kept, the others square to it.
fn squared(c: &[Point]) -> Vec<Point> {
    let centre = Point {
        x: c.iter().map(|p| p.x).sum::<f32>() / 4.0,
        y: c.iter().map(|p| p.y).sum::<f32>() / 4.0,
    };
    let along = {
        let (dx, dy) = (
            c[1].x - c[0].x + c[2].x - c[3].x,
            c[1].y - c[0].y + c[2].y - c[3].y,
        );
        let l = dx.hypot(dy).max(f32::EPSILON);
        (dx / l, dy / l)
    };
    let across = (-along.1, along.0);
    let project =
        |p: Point, axis: (f32, f32)| (p.x - centre.x) * axis.0 + (p.y - centre.y) * axis.1;
    let half_w = c.iter().map(|p| project(*p, along).abs()).sum::<f32>() / 4.0;
    let half_h = c.iter().map(|p| project(*p, across).abs()).sum::<f32>() / 4.0;
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .into_iter()
        .map(|(s, t)| Point {
            x: centre.x + along.0 * half_w * s + across.0 * half_h * t,
            y: centre.y + along.1 * half_w * s + across.1 * half_h * t,
        })
        .collect()
}

/// The ellipse through a closed stroke: its centre, and its axes from the points' spread (a point evenly round an
/// ellipse of radius `r` along an axis spreads `r²/2` along it).
fn fit_ellipse(points: &[Point]) -> Option<Shape> {
    let n = points.len() as f32;
    let cx = points.iter().map(|p| p.x).sum::<f32>() / n;
    let cy = points.iter().map(|p| p.y).sum::<f32>() / n;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for p in points {
        let (dx, dy) = (p.x - cx, p.y - cy);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let (sxx, syy, sxy) = (sxx / n, syy / n, sxy / n);
    let mean = (sxx + syy) / 2.0;
    let spread = (((sxx - syy) / 2.0).powi(2) + sxy * sxy).sqrt();
    let (l1, l2) = (mean + spread, mean - spread);
    if l2 <= 0.0 {
        return None;
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    Some(Shape::Ellipse {
        centre: Point { x: cx, y: cy },
        rx: (2.0 * l1).sqrt(),
        ry: (2.0 * l2).sqrt(),
        angle,
    })
}

/// How far `points` lie from `ellipse`, on average, along the ray from its centre.
fn ellipse_distance(points: &[Point], ellipse: &Shape) -> f32 {
    let Shape::Ellipse {
        centre,
        rx,
        ry,
        angle,
    } = ellipse
    else {
        return f32::INFINITY;
    };
    let (sin, cos) = angle.sin_cos();
    let total: f32 = points
        .iter()
        .map(|p| {
            let (dx, dy) = (p.x - centre.x, p.y - centre.y);
            let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            let r = u.hypot(v);
            if r <= f32::EPSILON {
                return rx.min(*ry);
            }
            // The ellipse's radius in this direction.
            let (cu, sv) = (u / r, v / r);
            let edge = 1.0 / ((cu / rx).powi(2) + (sv / ry).powi(2)).sqrt();
            (r - edge).abs()
        })
        .sum();
    total / points.len() as f32
}

/// Douglas-Peucker: the points of `points` that keep its shape within `tolerance`, its ends always.
fn simplify(points: &[Point], tolerance: f32) -> Vec<Point> {
    fn split(points: &[Point], tolerance: f32, out: &mut Vec<Point>) {
        let (first, last) = (points[0], points[points.len() - 1]);
        let far = points[1..points.len() - 1]
            .iter()
            .enumerate()
            .map(|(i, p)| (i + 1, to_segment(*p, first, last)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match far {
            Some((at, d)) if d > tolerance => {
                split(&points[..=at], tolerance, out);
                split(&points[at..], tolerance, out);
            }
            _ => out.push(last),
        }
    }
    let mut out = vec![points[0]];
    if points.len() >= 2 {
        split(points, tolerance, &mut out);
    }
    out
}

/// The mean distance from `points` to the polyline through `corners`, closed back to its start when `closed`.
fn mean_distance(points: &[Point], corners: &[Point], closed: bool) -> f32 {
    let segments: Vec<(Point, Point)> = corners
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(closed.then(|| (corners[corners.len() - 1], corners[0])))
        .collect();
    let total: f32 = points
        .iter()
        .map(|p| {
            segments
                .iter()
                .map(|(a, b)| to_segment(*p, *a, *b))
                .fold(f32::INFINITY, f32::min)
        })
        .sum();
    total / points.len() as f32
}

/// An arrowhead's barbs at `tip`, pointing back along the shaft from `tail`: a quarter of it long, 30 degrees out.
fn barbs(tail: Point, tip: Point, shaft: f32) -> [Point; 2] {
    let length = (shaft * 0.25).max(1.0);
    let back = ((tail.x - tip.x) / shaft, (tail.y - tip.y) / shaft);
    let turned = |degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Point {
            x: tip.x + (back.0 * cos - back.1 * sin) * length,
            y: tip.y + (back.0 * sin + back.1 * cos) * length,
        }
    };
    [turned(30.0), turned(-30.0)]
}

fn to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let squared = dx * dx + dy * dy;
    let t = if squared > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(
        p,
        Point {
            x: a.x + dx * t,
            y: a.y + dy * t,
        },
    )
}

fn index_of(points: &[Point], target: Point) -> usize {
    points
        .iter()
        .position(|p| *p == target)
        .unwrap_or(points.len() - 1)
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn midpoint(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}

fn path_length(points: &[Point]) -> f32 {
    points.windows(2).map(|w| distance(w[0], w[1])).sum()
}

fn diagonal(points: &[Point]) -> f32 {
    let (x0, x1) = points
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y0, y1) = points
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    (x1 - x0).hypot(y1 - y0)
}

#[cfg(test)]
mod tests;
