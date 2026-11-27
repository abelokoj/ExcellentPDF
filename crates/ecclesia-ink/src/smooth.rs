//! Smoothing (plan §8.2, step 4): light filtering of the samples, then a Catmull-Rom spline through them as cubic
//! Béziers (B0 = P1, B1 = P1 + (P2 − P0) / 6, B2 = P2 − (P3 − P1) / 6, B3 = P2), and those curves flattened back to
//! points as finely as asked, for readers that only take points (a PDF ink list) and for drawing.

use crate::stroke::InkPoint;

/// One cubic Bézier of a smoothed stroke; the width goes from `from`'s to `to`'s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Where it starts.
    pub from: InkPoint,
    /// The first control point.
    pub c1: (f32, f32),
    /// The second control point.
    pub c2: (f32, f32),
    /// Where it ends.
    pub to: InkPoint,
}

impl Segment {
    /// The point `t` (0 to 1) along it.
    #[must_use]
    pub fn at(&self, t: f32) -> InkPoint {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        InkPoint {
            x: a * self.from.x + b * self.c1.0 + c * self.c2.0 + d * self.to.x,
            y: a * self.from.y + b * self.c1.1 + c * self.c2.1 + d * self.to.y,
            width: u * self.from.width + t * self.to.width,
        }
    }

    /// How many straight pieces keep it within `tolerance` of the curve (Wang's formula for a cubic).
    fn pieces(&self, tolerance: f32) -> usize {
        let second = |p0: f32, p1: f32, p2: f32| p0 - 2.0 * p1 + p2;
        let bend = |(ax, ay): (f32, f32), (bx, by): (f32, f32), (cx, cy): (f32, f32)| {
            second(ax, bx, cx).hypot(second(ay, by, cy))
        };
        let from = (self.from.x, self.from.y);
        let to = (self.to.x, self.to.y);
        let most = bend(from, self.c1, self.c2).max(bend(self.c1, self.c2, to));
        let pieces = (0.75 * most / tolerance.max(f32::EPSILON)).sqrt().ceil();
        // shortcut: at most 64 pieces a segment; samples are a few points apart, so no real stroke comes near it.
        (pieces as usize).clamp(1, 64)
    }
}

/// The Catmull-Rom spline through `points` as Béziers, one from each point to the next; the ends repeat their point.
#[must_use]
pub fn catmull_rom(points: &[InkPoint]) -> Vec<Segment> {
    let last = points.len().saturating_sub(1);
    (0..last)
        .map(|i| {
            let p0 = points[i.saturating_sub(1)];
            let (p1, p2) = (points[i], points[i + 1]);
            let p3 = points[(i + 2).min(last)];
            Segment {
                from: p1,
                c1: (p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0),
                c2: (p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0),
                to: p2,
            }
        })
        .collect()
}

/// `segments` as points, none further than `tolerance` from the curve: every segment's start, the points between,
/// and the last one's end.
#[must_use]
pub fn flatten(segments: &[Segment], tolerance: f32) -> Vec<InkPoint> {
    let mut points = Vec::new();
    for segment in segments {
        let pieces = segment.pieces(tolerance);
        points.extend((0..pieces).map(|i| segment.at(i as f32 / pieces as f32)));
    }
    points.extend(segments.last().map(|segment| segment.to));
    points
}

/// `points` filtered against hand tremor: each one moved `strength` (0 none, towards 1 heavy) of the way back towards
/// the filtered one before it. The first and last stay where they are, so a stroke still starts and ends where the
/// pen did.
#[must_use]
pub fn filter(points: &[InkPoint], strength: f32) -> Vec<InkPoint> {
    let strength = strength.clamp(0.0, 0.95);
    let mut filtered: Vec<InkPoint> = Vec::with_capacity(points.len());
    for (i, &point) in points.iter().enumerate() {
        let next = match filtered.last() {
            Some(previous) if i + 1 < points.len() => InkPoint {
                x: point.x + (previous.x - point.x) * strength,
                y: point.y + (previous.y - point.y) * strength,
                width: point.width,
            },
            _ => point,
        };
        filtered.push(next);
    }
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> InkPoint {
        InkPoint { x, y, width: 1.0 }
    }

    #[test]
    fn the_spline_passes_through_every_point() {
        let points = [at(0.0, 0.0), at(10.0, 5.0), at(20.0, 0.0), at(30.0, 5.0)];
        let segments = catmull_rom(&points);
        assert_eq!(segments.len(), 3);
        for (segment, pair) in segments.iter().zip(points.windows(2)) {
            assert_eq!(segment.at(0.0), pair[0]);
            assert_eq!(segment.at(1.0), pair[1]);
        }
        // The plan's control points: B1 = P1 + (P2 − P0) / 6 for the middle segment.
        assert_eq!(segments[1].c1, (10.0 + 20.0 / 6.0, 5.0));
        assert!(catmull_rom(&points[..1]).is_empty());
    }

    #[test]
    fn a_straight_line_stays_straight() {
        let points = [at(0.0, 0.0), at(5.0, 0.0), at(10.0, 0.0)];
        let flat = flatten(&catmull_rom(&points), 0.05);
        assert!(flat.iter().all(|p| p.y == 0.0));
        assert_eq!(flat.first(), points.first());
        assert_eq!(flat.last(), points.last());
    }

    #[test]
    fn flattening_stays_within_the_tolerance() {
        let points = [at(0.0, 0.0), at(20.0, 30.0), at(40.0, 0.0), at(60.0, 30.0)];
        let segments = catmull_rom(&points);
        let flat = flatten(&segments, 0.1);
        assert!(flat.len() > points.len());
        // Every point of the curve lies near a straight piece between two flattened points.
        for segment in &segments {
            for step in 0..=100 {
                let p = segment.at(step as f32 / 100.0);
                let near = flat
                    .windows(2)
                    .any(|pair| distance_to_line(p, pair[0], pair[1]) <= 0.1 + 1e-3);
                assert!(near, "({}, {}) is off the flattened line", p.x, p.y);
            }
        }
    }

    fn distance_to_line(p: InkPoint, a: InkPoint, b: InkPoint) -> f32 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length = dx * dx + dy * dy;
        let t = if length == 0.0 {
            0.0
        } else {
            (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0)
        };
        (p.x - (a.x + t * dx)).hypot(p.y - (a.y + t * dy))
    }

    #[test]
    fn filtering_steadies_the_middle_and_keeps_the_ends() {
        let shaky = [
            at(0.0, 0.0),
            at(1.0, 1.0),
            at(2.0, -1.0),
            at(3.0, 1.0),
            at(4.0, 0.0),
        ];
        let steady = filter(&shaky, 0.5);
        assert_eq!(steady.first(), shaky.first());
        assert_eq!(steady.last(), shaky.last());
        let wobble = |points: &[InkPoint]| points.iter().map(|p| p.y.abs()).sum::<f32>();
        assert!(wobble(&steady) < wobble(&shaky));
        assert_eq!(filter(&shaky, 0.0), shaky);
    }
}
