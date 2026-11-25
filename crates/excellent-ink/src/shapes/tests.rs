//! H.10's "Done when": recognition accuracy is at least 95 % on a set of 500 shapes. The set is drawn the way a hand
//! draws: a random size, place and turn, points every few points along the outline, a wobble that drifts slowly and
//! a jitter on top, closed figures started anywhere and closed a little short or a little past their start, arrows
//! drawn shaft first with the head's barbs retraced from the tip. Seeded, so the same set every run.

use super::*;

/// A small seeded generator (SplitMix64): the same shapes on every run.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// `outline` (a closed one when `closed`) walked as a hand draws it: resampled every 2 to 4 points, a slow wobble and
/// a jitter of `noise` times the size.
fn hand(outline: &[Point], closed: bool, size: f32, noise: f32, rng: &mut Seeded) -> Vec<Point> {
    let mut ring: Vec<Point> = outline.to_vec();
    if closed {
        // Start anywhere along it, and stop a little short of the start or a little past it.
        let start = (rng.next() * ring.len() as f32) as usize % ring.len();
        ring.rotate_left(start);
        let first = ring[0];
        ring.push(first);
    }
    let mut points = Vec::new();
    let (phase, wave) = (rng.range(0.0, std::f32::consts::TAU), rng.range(0.5, 2.0));
    let mut walked = 0.0_f32;
    for pair in ring.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = distance(a, b);
        let mut t = 0.0;
        while t < length {
            let along = t / length.max(f32::EPSILON);
            let p = Point {
                x: a.x + (b.x - a.x) * along,
                y: a.y + (b.y - a.y) * along,
            };
            let drift = (walked / size * wave + phase).sin() * noise * size * 0.6;
            points.push(Point {
                x: p.x + drift + rng.range(-1.0, 1.0) * noise * size * 0.4,
                y: p.y - drift + rng.range(-1.0, 1.0) * noise * size * 0.4,
            });
            let step = rng.range(2.0, 4.0);
            t += step;
            walked += step;
        }
    }
    if closed {
        let overshoot = (rng.range(-0.06, 0.06) * points.len() as f32) as isize;
        if overshoot < 0 {
            points.truncate((points.len() as isize + overshoot) as usize);
        } else {
            let extra: Vec<Point> = points[..overshoot as usize].to_vec();
            points.extend(extra);
        }
    } else if let Some(last) = ring.last() {
        points.push(*last);
    }
    points
}

/// A shape of `kind`, as a hand draws it.
fn drawn(kind: Kind, rng: &mut Seeded) -> Vec<Point> {
    let size = rng.range(60.0, 250.0);
    let centre = Point {
        x: rng.range(150.0, 450.0),
        y: rng.range(150.0, 650.0),
    };
    let turn = rng.range(0.0, std::f32::consts::TAU);
    let place = |x: f32, y: f32| {
        let (sin, cos) = turn.sin_cos();
        Point {
            x: centre.x + x * cos - y * sin,
            y: centre.y + x * sin + y * cos,
        }
    };
    let noise = rng.range(0.006, 0.028);
    let half = size / 2.0;
    match kind {
        Kind::Line => hand(
            &[place(-half, 0.0), place(half, 0.0)],
            false,
            size,
            noise,
            rng,
        ),
        Kind::Arrow => {
            let (tail, tip) = (place(-half, 0.0), place(half, 0.0));
            let barb = size * rng.range(0.15, 0.3);
            let spread = rng.range(0.4, 0.7);
            let b1 = place(half - barb, barb * spread);
            let b2 = place(half - barb, -barb * spread);
            hand(&[tail, tip, b1, tip, b2], false, size, noise, rng)
        }
        Kind::Triangle => {
            let corners: Vec<Point> = (0..3)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 3.0 + rng.range(-0.2, 0.2);
                    place(a.cos() * half, a.sin() * half)
                })
                .collect();
            hand(&corners, true, size, noise, rng)
        }
        Kind::Rectangle => {
            let h = half * rng.range(0.45, 1.0);
            hand(
                &[
                    place(-half, -h),
                    place(half, -h),
                    place(half, h),
                    place(-half, h),
                ],
                true,
                size,
                noise,
                rng,
            )
        }
        Kind::Polygon => {
            let corners: Vec<Point> = (0..5)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 5.0;
                    place(a.cos() * half, a.sin() * half)
                })
                .collect();
            hand(&corners, true, size, noise, rng)
        }
        Kind::Circle | Kind::Ellipse => {
            let ry = if kind == Kind::Circle {
                half
            } else {
                half * rng.range(0.4, 0.7)
            };
            let ring: Vec<Point> = (0..96)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 96.0;
                    place(a.cos() * half, a.sin() * ry)
                })
                .collect();
            hand(&ring, true, size, noise, rng)
        }
    }
}

#[test]
fn recognizes_at_least_95_percent_of_500_hand_drawn_shapes() {
    let kinds = [
        Kind::Line,
        Kind::Arrow,
        Kind::Triangle,
        Kind::Rectangle,
        Kind::Polygon,
        Kind::Circle,
        Kind::Ellipse,
    ];
    let mut rng = Seeded(0x00e1_1e57);
    let mut misses: Vec<(Kind, Option<Kind>)> = Vec::new();
    let total = 500;
    for n in 0..total {
        let kind = kinds[n % kinds.len()];
        let got = recognize(&drawn(kind, &mut rng)).map(|shape| shape.kind());
        if got != Some(kind) {
            misses.push((kind, got));
        }
    }
    let accuracy = 1.0 - misses.len() as f32 / total as f32;
    eprintln!("shapes: {:.1} % of {total} right", accuracy * 100.0);
    assert!(
        accuracy >= 0.95,
        "{:.1} % right; missed: {misses:?}",
        accuracy * 100.0
    );
}

#[test]
fn a_scribble_stays_a_scribble() {
    let mut rng = Seeded(7);
    let scribble: Vec<Point> = (0..120)
        .map(|_| Point {
            x: rng.range(100.0, 300.0),
            y: rng.range(100.0, 300.0),
        })
        .collect();
    assert_eq!(recognize(&scribble), None);
}

#[test]
fn shapes_draw_true() {
    let rect = Shape::Polygon(vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 10.0, y: 0.0 },
        Point { x: 10.0, y: 5.0 },
        Point { x: 0.0, y: 5.0 },
    ]);
    assert_eq!(rect.kind(), Kind::Rectangle);
    assert_eq!(rect.path().first(), rect.path().last(), "closed");
    let circle = Shape::Ellipse {
        centre: Point { x: 0.0, y: 0.0 },
        rx: 10.0,
        ry: 10.0,
        angle: 0.0,
    };
    assert_eq!(circle.kind(), Kind::Circle);
    assert!(
        circle
            .path()
            .iter()
            .all(|p| (p.x.hypot(p.y) - 10.0).abs() < 1e-3)
    );
}
