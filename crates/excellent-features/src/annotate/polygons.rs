//! The geometry of polygons and polylines (A.10): a regular polygon or a star fitted in a box, as SumatraPDF Enhanced
//! draws them (`PolygonSides`, `StarSides`), and how far a point is from a path. Pure: page points in, page points
//! out.

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::ops::RangeInclusive;

use excellent_mupdf::{PagePoint, PageRect};

/// The side counts of a regular polygon, and the point counts of a star, that the tools offer.
pub const SIDES: RangeInclusive<u8> = 3..=32;

/// A regular polygon's sides, and a star's points, before the reader chooses.
pub const DEFAULT_SIDES: u8 = 5;

/// A star's inner corners, as a share of its outer radius: Enhanced's.
const STAR_INNER: f32 = 0.45;

/// `n` points round the ellipse that fills `rect`, the first at the top, `turn` radians apart, each scaled out from
/// the centre by `radius(i)`.
fn round(rect: PageRect, n: usize, turn: f32, radius: impl Fn(usize) -> f32) -> Vec<PagePoint> {
    let (cx, cy) = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    let (rx, ry) = ((rect.x1 - rect.x0) / 2.0, (rect.y1 - rect.y0) / 2.0);
    (0..n)
        .map(|i| {
            let angle = -FRAC_PI_2 + turn * i as f32;
            PagePoint {
                x: cx + rx * radius(i) * angle.cos(),
                y: cy + ry * radius(i) * angle.sin(),
            }
        })
        .collect()
}

/// The corners of a regular polygon of `sides` sides (clamped to [`SIDES`]) fitted in `rect`, the first at the top.
#[must_use]
pub fn regular(rect: PageRect, sides: u8) -> Vec<PagePoint> {
    let n = usize::from(sides.clamp(*SIDES.start(), *SIDES.end()));
    round(rect, n, TAU / n as f32, |_| 1.0)
}

/// The corners of a star of `points` points (clamped to [`SIDES`]) fitted in `rect`: a tip at the top, then inner and
/// outer corners in turn.
#[must_use]
pub fn star(rect: PageRect, points: u8) -> Vec<PagePoint> {
    let n = usize::from(points.clamp(*SIDES.start(), *SIDES.end()));
    round(rect, 2 * n, PI / n as f32, |i| {
        if i % 2 == 0 { 1.0 } else { STAR_INNER }
    })
}

/// `points` without a point that repeats the one before it (within `close` points): a double-click puts its point
/// down twice.
#[must_use]
pub fn deduped(points: &[PagePoint], close: f32) -> Vec<PagePoint> {
    let mut kept: Vec<PagePoint> = Vec::with_capacity(points.len());
    for &point in points {
        if kept
            .last()
            .is_none_or(|last| (point.x - last.x).hypot(point.y - last.y) > close)
        {
            kept.push(point);
        }
    }
    kept
}

/// The smallest box round `points`; `None` for none.
#[must_use]
pub fn bounds(points: &[PagePoint]) -> Option<PageRect> {
    let first = points.first()?;
    Some(points.iter().fold(
        PageRect {
            x0: first.x,
            y0: first.y,
            x1: first.x,
            y1: first.y,
        },
        |rect, point| PageRect {
            x0: rect.x0.min(point.x),
            y0: rect.y0.min(point.y),
            x1: rect.x1.max(point.x),
            y1: rect.y1.max(point.y),
        },
    ))
}

/// The segments of the path through `points`, and back to the first when it is `closed`.
fn segments(
    points: &[PagePoint],
    closed: bool,
) -> impl Iterator<Item = (PagePoint, PagePoint)> + '_ {
    let back = (closed && points.len() > 2)
        .then(|| points.last().copied().zip(points.first().copied()))
        .flatten();
    points.windows(2).map(|pair| (pair[0], pair[1])).chain(back)
}

/// How far `point` is from the path through `points`, closed or not, in points; infinite for no points.
#[must_use]
pub fn distance(points: &[PagePoint], closed: bool, point: PagePoint) -> f32 {
    match points {
        [] => f32::INFINITY,
        [only] => (point.x - only.x).hypot(point.y - only.y),
        _ => segments(points, closed)
            .map(|(from, to)| super::shapes::distance(from, to, point))
            .fold(f32::INFINITY, f32::min),
    }
}

/// Whether `point` is inside the polygon through `points` (even-odd, as a PDF fills it).
#[must_use]
pub fn inside(points: &[PagePoint], point: PagePoint) -> bool {
    segments(points, true)
        .filter(|(a, b)| (a.y > point.y) != (b.y > point.y))
        .filter(|(a, b)| point.x < a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y))
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 100.0,
        y1: 100.0,
    };

    fn at(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    fn close(a: PagePoint, b: PagePoint) -> bool {
        (a.x - b.x).hypot(a.y - b.y) < 0.01
    }

    #[test]
    fn a_regular_polygon_and_a_star_fit_their_box_tip_up() {
        let pentagon = regular(BOX, 5);
        assert_eq!(pentagon.len(), 5);
        assert!(
            close(pentagon[0], at(50.0, 0.0)),
            "the first corner at the top"
        );
        assert!(
            pentagon
                .iter()
                .all(|p| ((p.x - 50.0).hypot(p.y - 50.0) - 50.0).abs() < 0.01)
        );
        let star = star(BOX, 5);
        assert_eq!(star.len(), 10);
        assert!(close(star[0], at(50.0, 0.0)));
        assert!(
            ((star[1].x - 50.0).hypot(star[1].y - 50.0) - 22.5).abs() < 0.01,
            "an inner corner"
        );
        assert_eq!(regular(BOX, 1).len(), 3, "at least a triangle");
        assert_eq!(regular(BOX, 200).len(), 32);
    }

    #[test]
    fn a_double_click_s_second_point_is_dropped() {
        let points = [at(0.0, 0.0), at(10.0, 0.0), at(10.2, 0.1), at(10.0, 10.0)];
        assert_eq!(
            deduped(&points, 0.5),
            vec![at(0.0, 0.0), at(10.0, 0.0), at(10.0, 10.0)]
        );
        assert_eq!(
            bounds(&points),
            Some(PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 10.2,
                y1: 10.0
            })
        );
        assert_eq!(bounds(&[]), None);
    }

    #[test]
    fn distance_and_inside_follow_the_path() {
        let triangle = [at(0.0, 0.0), at(100.0, 0.0), at(0.0, 100.0)];
        assert!((distance(&triangle, false, at(50.0, 3.0)) - 3.0).abs() < 0.01);
        assert!(
            distance(&triangle, false, at(-3.0, 50.0)) > 3.0,
            "an open path has no closing side"
        );
        assert!((distance(&triangle, true, at(-3.0, 50.0)) - 3.0).abs() < 0.01);
        assert!(inside(&triangle, at(20.0, 20.0)));
        assert!(!inside(&triangle, at(80.0, 80.0)));
        assert!(distance(&[], true, at(0.0, 0.0)).is_infinite());
    }
}
