//! Drawing guides (H.11, plan §8.4): a ruler, a protractor and a compass laid on the page. A stroke drawn along the
//! ruler's edge comes out exactly straight on it; one drawn round the compass comes out an exact arc; one drawn out
//! from the protractor's centre comes out straight at a whole degree. Each says what was drawn: a length, an angle, a
//! radius. All in page points.

use crate::erase::Point;

/// How far a stroke may stray from a guide, in page points, and still be drawn along it.
pub const REACH: f32 = 6.0;

/// Points on an arc a compass draws, per radian.
const ARC_STEPS_PER_RADIAN: f32 = 24.0;

/// A guide on a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    /// A straight edge from `origin`, `angle` radians clockwise from across the page, `length` long.
    Ruler {
        /// Where its edge starts.
        origin: Point,
        /// Its direction.
        angle: f32,
        /// Its length.
        length: f32,
    },
    /// A half circle of angles about `centre`, its base `angle` radians from across the page.
    Protractor {
        /// Its centre.
        centre: Point,
        /// Its base's direction.
        angle: f32,
        /// Its radius.
        radius: f32,
    },
    /// A circle of `radius` about `centre`.
    Compass {
        /// Its centre (the point).
        centre: Point,
        /// Its radius (the pencil's reach).
        radius: f32,
    },
}

/// What a stroke drawn along a guide became, and what it measures.
#[derive(Debug, Clone, PartialEq)]
pub struct Guided {
    /// The stroke as drawn true along the guide.
    pub path: Vec<Point>,
    /// What it measures: a line's length in points, a line's angle in degrees, an arc's radius and sweep.
    pub reading: Reading,
}

/// A measurement shown beside the guide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    /// A straight line: its length in points and its angle in degrees clockwise from across the page.
    Line {
        /// Its length.
        length: f32,
        /// Its angle.
        degrees: f32,
    },
    /// An arc: its radius in points and how far round it goes, in degrees.
    Arc {
        /// Its radius.
        radius: f32,
        /// Its sweep.
        degrees: f32,
    },
}

impl Guide {
    /// `stroke` drawn true along this guide, when it lies along it (every point within [`REACH`]); `None` when it
    /// doesn't.
    #[must_use]
    pub fn guide(&self, stroke: &[Point]) -> Option<Guided> {
        let (first, last) = (*stroke.first()?, *stroke.last()?);
        match *self {
            Self::Ruler {
                origin,
                angle,
                length,
            } => {
                let (cos, sin) = (angle.cos(), angle.sin());
                let along = |p: Point| (p.x - origin.x) * cos + (p.y - origin.y) * sin;
                let across = |p: Point| -(p.x - origin.x) * sin + (p.y - origin.y) * cos;
                if !stroke.iter().all(|&p| across(p).abs() <= REACH) {
                    return None;
                }
                let on = |t: f32| {
                    let t = t.clamp(0.0, length);
                    Point {
                        x: origin.x + cos * t,
                        y: origin.y + sin * t,
                    }
                };
                let (a, b) = (on(along(first)), on(along(last)));
                Some(line(a, b))
            }
            Self::Protractor { centre, .. } => {
                // Out from the centre: straight, at a whole degree.
                if distance(first, centre) > REACH * 2.0 {
                    return None;
                }
                let reach = distance(centre, last);
                let raw = (last.y - centre.y).atan2(last.x - centre.x).to_degrees();
                let degrees = raw.round();
                let end = Point {
                    x: centre.x + degrees.to_radians().cos() * reach,
                    y: centre.y + degrees.to_radians().sin() * reach,
                };
                let straight = stroke.iter().all(|&p| to_line(p, centre, end) <= REACH);
                straight.then(|| line(centre, end))
            }
            Self::Compass { centre, radius } => {
                if !stroke
                    .iter()
                    .all(|&p| (distance(p, centre) - radius).abs() <= REACH)
                {
                    return None;
                }
                let start = (first.y - centre.y).atan2(first.x - centre.x);
                // How far round it went: the sum of each step's turn, so a full circle is a full circle.
                let sweep: f32 = stroke
                    .windows(2)
                    .map(|w| {
                        let a = (w[0].y - centre.y).atan2(w[0].x - centre.x);
                        let b = (w[1].y - centre.y).atan2(w[1].x - centre.x);
                        wrap(b - a)
                    })
                    .sum();
                let sweep = sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
                let steps = ((sweep.abs() * ARC_STEPS_PER_RADIAN).ceil() as usize).max(2);
                let path = (0..=steps)
                    .map(|i| {
                        let t = start + sweep * i as f32 / steps as f32;
                        Point {
                            x: centre.x + t.cos() * radius,
                            y: centre.y + t.sin() * radius,
                        }
                    })
                    .collect();
                Some(Guided {
                    path,
                    reading: Reading::Arc {
                        radius,
                        degrees: sweep.to_degrees().abs(),
                    },
                })
            }
        }
    }
}

fn line(a: Point, b: Point) -> Guided {
    Guided {
        path: vec![a, b],
        reading: Reading::Line {
            length: distance(a, b),
            degrees: (b.y - a.y).atan2(b.x - a.x).to_degrees(),
        },
    }
}

/// `angle` brought into -π to π.
fn wrap(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The distance from `p` to the infinite line through `a` and `b`.
fn to_line(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return distance(p, a);
    }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// A hand's line along a 30-degree ruler: wobbling a few points either side of the edge.
    fn wobbly_along(origin: Point, angle: f32, from: f32, to: f32) -> Vec<Point> {
        (0..=40)
            .map(|i| {
                let t = from + (to - from) * i as f32 / 40.0;
                let off = (i as f32 * 0.9).sin() * 3.0;
                p(
                    origin.x + angle.cos() * t - angle.sin() * off,
                    origin.y + angle.sin() * t + angle.cos() * off,
                )
            })
            .collect()
    }

    #[test]
    fn a_stroke_along_the_ruler_is_exactly_straight() {
        let angle = 30_f32.to_radians();
        let ruler = Guide::Ruler {
            origin: p(100.0, 100.0),
            angle,
            length: 400.0,
        };
        let guided = ruler
            .guide(&wobbly_along(p(100.0, 100.0), angle, 50.0, 250.0))
            .expect("along the edge");
        let [a, b] = guided.path[..] else {
            panic!("a straight line: {:?}", guided.path);
        };
        // On the edge itself, at 30 degrees, 200 points long.
        for q in [a, b] {
            let across = -(q.x - 100.0) * angle.sin() + (q.y - 100.0) * angle.cos();
            assert!(across.abs() < 1e-3, "on the edge: {across}");
        }
        let Reading::Line { length, degrees } = guided.reading else {
            panic!("a line");
        };
        assert!(
            (length - 200.0).abs() < 0.5 && (degrees - 30.0).abs() < 0.01,
            "{length} {degrees}"
        );
    }

    #[test]
    fn a_stroke_off_the_ruler_is_left_alone() {
        let ruler = Guide::Ruler {
            origin: p(0.0, 0.0),
            angle: 0.0,
            length: 300.0,
        };
        assert_eq!(ruler.guide(&[p(10.0, 20.0), p(100.0, 25.0)]), None);
    }

    #[test]
    fn a_stroke_round_the_compass_is_an_exact_arc() {
        let compass = Guide::Compass {
            centre: p(200.0, 200.0),
            radius: 50.0,
        };
        let stroke: Vec<Point> = (0..=30)
            .map(|i| {
                let t = (i as f32 * 3.0).to_radians();
                let r = 50.0 + (i as f32).sin() * 2.0;
                p(200.0 + t.cos() * r, 200.0 + t.sin() * r)
            })
            .collect();
        let guided = compass.guide(&stroke).expect("round the circle");
        assert!(
            guided
                .path
                .iter()
                .all(|q| ((q.x - 200.0).hypot(q.y - 200.0) - 50.0).abs() < 1e-3)
        );
        let Reading::Arc { degrees, .. } = guided.reading else {
            panic!("an arc");
        };
        assert!((degrees - 90.0).abs() < 0.5, "{degrees}");
    }

    #[test]
    fn a_stroke_from_the_protractor_lands_on_a_whole_degree() {
        let protractor = Guide::Protractor {
            centre: p(100.0, 100.0),
            angle: 0.0,
            radius: 120.0,
        };
        let t = 37.4_f32.to_radians();
        let stroke: Vec<Point> = (0..=10)
            .map(|i| {
                let r = i as f32 * 10.0;
                p(100.0 + t.cos() * r, 100.0 + t.sin() * r)
            })
            .collect();
        let guided = protractor.guide(&stroke).expect("out from the centre");
        let Reading::Line { degrees, .. } = guided.reading else {
            panic!("a line");
        };
        assert!((degrees - 37.0).abs() < 1e-3, "{degrees}");
    }
}
