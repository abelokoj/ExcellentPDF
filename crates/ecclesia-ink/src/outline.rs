//! A stroke's outline (H.9, plan §8.3): the filled shape a pressure pen leaves, its width changing point by point,
//! with round ends. A PDF ink annotation's appearance fills these outlines, so every reader draws the stroke as the
//! pen drew it, not as one constant-width line.

use crate::stroke::InkPoint;

/// How many straight pieces make each round end's half circle.
const CAP_STEPS: usize = 8;

/// Widths closer than this share, relative to the widest, are one width: such a stroke is a constant-width line, and
/// its appearance is the PDF reader's own.
const EVEN: f32 = 0.02;

/// Whether `points` all have one width (within 2 %): a ballpoint's or a marker's stroke, drawn as a plain line.
#[must_use]
pub fn is_even(points: &[InkPoint]) -> bool {
    let widest = points.iter().map(|p| p.width).fold(0.0, f32::max);
    let thinnest = points.iter().map(|p| p.width).fold(f32::INFINITY, f32::min);
    widest <= 0.0 || (widest - thinnest) <= widest * EVEN
}

/// The closed outline of the stroke through `points`, each as wide as its point says, with round ends: down one side
/// from the first point to the last, round the last end, back up the other side and round the first end. A single
/// point is a dot; no points, no outline.
#[must_use]
pub fn outline(points: &[InkPoint]) -> Vec<(f32, f32)> {
    let points = distinct(points);
    match points.as_slice() {
        [] => Vec::new(),
        [only] => circle(*only),
        _ => {
            let normals = normals(&points);
            let side = |sign: f32| {
                points.iter().zip(&normals).map(move |(p, (nx, ny))| {
                    let half = p.width / 2.0 * sign;
                    (p.x + nx * half, p.y + ny * half)
                })
            };
            let last = points.len() - 1;
            let (end_dir, start_dir) = (normals[last], normals[0]);
            let mut ring: Vec<(f32, f32)> = side(1.0).collect();
            ring.extend(cap(points[last], end_dir, 1.0));
            ring.extend(side(-1.0).rev());
            ring.extend(cap(points[0], start_dir, -1.0));
            ring
        }
    }
}

/// `points` without those that repeat the one before (no direction to take a normal from).
fn distinct(points: &[InkPoint]) -> Vec<InkPoint> {
    let mut kept: Vec<InkPoint> = Vec::with_capacity(points.len());
    for &p in points {
        match kept.last_mut() {
            Some(last) if (last.x - p.x).abs() < 1e-4 && (last.y - p.y).abs() < 1e-4 => {
                last.width = last.width.max(p.width);
            }
            _ => kept.push(p),
        }
    }
    kept
}

/// Each point's unit normal: across the direction the stroke runs there (the mean of the segments either side).
fn normals(points: &[InkPoint]) -> Vec<(f32, f32)> {
    let unit = |a: InkPoint, b: InkPoint| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length = dx.hypot(dy).max(f32::EPSILON);
        (dx / length, dy / length)
    };
    (0..points.len())
        .map(|i| {
            let before = (i > 0).then(|| unit(points[i - 1], points[i]));
            let after = (i + 1 < points.len()).then(|| unit(points[i], points[i + 1]));
            let (dx, dy) = match (before, after) {
                (Some(a), Some(b)) => {
                    let (x, y) = (a.0 + b.0, a.1 + b.1);
                    let length = x.hypot(y);
                    if length < 1e-3 {
                        b
                    } else {
                        (x / length, y / length)
                    }
                }
                (Some(d), None) | (None, Some(d)) => d,
                (None, None) => (1.0, 0.0),
            };
            (-dy, dx)
        })
        .collect()
}

/// The half circle round `point`'s end, from the side `sign` gives to the other, bulging along the stroke's direction
/// (`normal` turned back): the interior points only, as the sides give the ends.
fn cap(point: InkPoint, normal: (f32, f32), sign: f32) -> impl Iterator<Item = (f32, f32)> {
    let half = point.width / 2.0;
    // The direction the stroke runs at this end; outward is along it at the last point and against it at the first.
    let along = (normal.1, -normal.0);
    (1..CAP_STEPS).map(move |step| {
        let angle = std::f32::consts::PI * step as f32 / CAP_STEPS as f32;
        let (sin, cos) = angle.sin_cos();
        // From +normal (sign 1) through outward to -normal.
        let x = normal.0 * cos * sign + along.0 * sin * sign;
        let y = normal.1 * cos * sign + along.1 * sin * sign;
        (point.x + x * half, point.y + y * half)
    })
}

/// A dot: `point`'s width across.
fn circle(point: InkPoint) -> Vec<(f32, f32)> {
    let half = point.width / 2.0;
    (0..CAP_STEPS * 2)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / (CAP_STEPS * 2) as f32;
            (point.x + angle.cos() * half, point.y + angle.sin() * half)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32, width: f32) -> InkPoint {
        InkPoint { x, y, width }
    }

    /// Twice the polygon's signed area (shoelace).
    fn area(ring: &[(f32, f32)]) -> f32 {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
            .sum::<f32>()
            .abs()
            / 2.0
    }

    #[test]
    fn a_tapering_stroke_is_as_wide_as_each_point() {
        let ring = outline(&[p(0.0, 0.0, 2.0), p(10.0, 0.0, 6.0)]);
        // The sides: at x = 0 one point each side, 1 from the centre line; at x = 10, 3.
        assert!(
            ring.contains(&(0.0, 1.0)) && ring.contains(&(0.0, -1.0)),
            "{ring:?}"
        );
        assert!(
            ring.contains(&(10.0, 3.0)) && ring.contains(&(10.0, -3.0)),
            "{ring:?}"
        );
        // A trapezoid of area 40 plus half circles of radius 1 and 3: about 40 + 1.57 + 14.1.
        let expected = 40.0 + std::f32::consts::PI * (1.0 + 9.0) / 2.0;
        assert!(
            (area(&ring) - expected).abs() < 1.0,
            "{} vs {expected}",
            area(&ring)
        );
    }

    #[test]
    fn the_ends_bulge_outwards() {
        let ring = outline(&[p(0.0, 0.0, 4.0), p(10.0, 0.0, 4.0)]);
        let (min_x, max_x) = ring.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (x, _)| {
            (lo.min(*x), hi.max(*x))
        });
        assert!(
            (min_x + 2.0).abs() < 0.05 && (max_x - 12.0).abs() < 0.05,
            "{min_x} {max_x}"
        );
    }

    #[test]
    fn a_dot_is_round_and_nothing_is_nothing() {
        let dot = outline(&[p(5.0, 5.0, 2.0), p(5.0, 5.0, 2.0)]);
        assert_eq!(dot.len(), CAP_STEPS * 2);
        assert!((area(&dot) - std::f32::consts::PI).abs() < 0.2);
        assert!(outline(&[]).is_empty());
    }

    #[test]
    fn even_strokes_are_told_apart_from_pressure_strokes() {
        assert!(is_even(&[p(0.0, 0.0, 3.0), p(1.0, 0.0, 3.03)]));
        assert!(!is_even(&[p(0.0, 0.0, 1.0), p(1.0, 0.0, 3.0)]));
    }
}
