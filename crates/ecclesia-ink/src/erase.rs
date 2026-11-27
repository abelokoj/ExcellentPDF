//! Erasers (H.5, plan §8.4): what an eraser's path does to the strokes it crosses. The stroke eraser removes every
//! stroke it touches, the area eraser cuts away only the parts under it (one stroke can come out as two), and the
//! highlighter eraser removes only highlighter ink. Pure geometry in page points; the app turns the result into one
//! step of undo.
//!
//! The eraser's path with its radius is a chain of capsules. A capsule is convex, so each stroke segment meets it in
//! one interval, found exactly enough by searching the distance along the segment, which is convex too.

/// A point in page points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Across.
    pub x: f32,
    /// Down.
    pub y: f32,
}

impl Point {
    fn minus(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            x: self.x + (to.x - self.x) * t,
            y: self.y + (to.y - self.y) * t,
        }
    }

    fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

/// Which eraser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Eraser {
    /// Removes each stroke it touches, whole.
    #[default]
    Stroke,
    /// Removes only the parts of strokes under it, splitting them.
    Area,
    /// Removes each highlighter stroke it touches, whole, and leaves other ink.
    Highlighter,
}

impl Eraser {
    /// Every eraser, in the toolbar's order.
    pub const ALL: [Self; 3] = [Self::Stroke, Self::Area, Self::Highlighter];

    /// Its name in settings, commands and events.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Stroke => "stroke",
            Self::Area => "area",
            Self::Highlighter => "highlighter",
        }
    }

    /// The eraser named `name` ([`Eraser::name`]).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|eraser| eraser.name() == name)
    }
}

/// A stroke as the eraser sees it.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// Its centre line.
    pub path: &'a [Point],
    /// Its width, in page points: the eraser reaches its edge, not only its centre.
    pub width: f32,
    /// Whether it is highlighter ink (H.3).
    pub highlighter: bool,
}

/// What an eraser did to one stroke.
#[derive(Debug, Clone, PartialEq)]
pub enum Cut {
    /// Not touched.
    Kept,
    /// Gone.
    Removed,
    /// What is left of it: two or more pieces, or one shorter piece.
    Split(Vec<Vec<Point>>),
}

/// An eraser's pass: its path in page points and its radius.
#[derive(Debug, Clone)]
pub struct Pass {
    eraser: Eraser,
    path: Vec<Point>,
    radius: f32,
}

/// The shortest piece the area eraser leaves, in page points: crumbs shorter than this go too.
const MIN_PIECE: f32 = 0.5;

/// How many halvings each interval search takes: 2⁻²⁴ of a segment, below a float's precision on a page.
const SEARCH_STEPS: u32 = 24;

impl Pass {
    /// `eraser` along `path` (page points, in the order drawn) with `radius` page points. Points closer than a
    /// quarter radius to the last one kept are dropped: they add nothing the capsules don't cover.
    #[must_use]
    pub fn new(eraser: Eraser, path: &[Point], radius: f32) -> Self {
        let mut kept: Vec<Point> = Vec::with_capacity(path.len());
        for &point in path {
            if kept
                .last()
                .is_none_or(|last| point.minus(*last).length() >= radius / 4.0)
            {
                kept.push(point);
            }
        }
        Self {
            eraser,
            path: kept,
            radius: radius.max(0.0),
        }
    }

    /// What this pass does to `target`.
    #[must_use]
    pub fn cut(&self, target: Target<'_>) -> Cut {
        if self.eraser == Eraser::Highlighter && !target.highlighter {
            return Cut::Kept;
        }
        let reach = self.radius + target.width / 2.0;
        let removed: Vec<Vec<(f32, f32)>> = segments(target.path)
            .map(|(a, b)| self.covered(a, b, reach))
            .collect();
        if removed.iter().all(Vec::is_empty) {
            return Cut::Kept;
        }
        match self.eraser {
            Eraser::Stroke | Eraser::Highlighter => Cut::Removed,
            Eraser::Area => match pieces(target.path, &removed) {
                pieces if pieces.is_empty() => Cut::Removed,
                pieces => Cut::Split(pieces),
            },
        }
    }

    /// The parts of segment `a`–`b` within `reach` of the eraser's path, as sorted, disjoint intervals of its
    /// parameter (0 at `a`, 1 at `b`).
    fn covered(&self, a: Point, b: Point, reach: f32) -> Vec<(f32, f32)> {
        let mut hits: Vec<(f32, f32)> = self
            .capsules()
            .filter(|&(p, q)| boxes_meet(a, b, p, q, reach))
            .filter_map(|(p, q)| interval(a, b, p, q, reach))
            .collect();
        hits.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(hits.len());
        for (start, end) in hits {
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
    }

    /// The eraser's capsules: one per segment of its path, or one point for a tap.
    fn capsules(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let tap = (self.path.len() == 1).then(|| (self.path[0], self.path[0]));
        tap.into_iter().chain(segments(&self.path))
    }
}

/// Each segment of `path`; a single point is a segment of no length.
fn segments(path: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    let single = (path.len() == 1).then(|| (path[0], path[0]));
    single
        .into_iter()
        .chain(path.windows(2).map(|pair| (pair[0], pair[1])))
}

/// Whether the boxes of `a`–`b` and of `p`–`q` grown by `reach` overlap: a cheap test before the exact one.
fn boxes_meet(a: Point, b: Point, p: Point, q: Point, reach: f32) -> bool {
    let apart = |lo1: f32, hi1: f32, lo2: f32, hi2: f32| hi1 + reach < lo2 || hi2 + reach < lo1;
    !(apart(a.x.min(b.x), a.x.max(b.x), p.x.min(q.x), p.x.max(q.x))
        || apart(a.y.min(b.y), a.y.max(b.y), p.y.min(q.y), p.y.max(q.y)))
}

/// The distance from `point` to segment `p`–`q`.
fn distance_to_segment(point: Point, p: Point, q: Point) -> f32 {
    let along = q.minus(p);
    let squared = along.x * along.x + along.y * along.y;
    let t = if squared > 0.0 {
        let to = point.minus(p);
        ((to.x * along.x + to.y * along.y) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.minus(p.lerp(q, t)).length()
}

/// The parameters of segment `a`–`b` within `reach` of segment `p`–`q`: one interval, since the capsule is convex
/// and the distance along a segment to it is convex too; `None` when they don't meet.
fn interval(a: Point, b: Point, p: Point, q: Point, reach: f32) -> Option<(f32, f32)> {
    let gap = |t: f32| distance_to_segment(a.lerp(b, t), p, q) - reach;
    // The closest parameter, by ternary search on the convex distance.
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..SEARCH_STEPS * 2 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if gap(m1) <= gap(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let nearest = (lo + hi) / 2.0;
    if gap(nearest) > 0.0 {
        return None;
    }
    // Each edge by bisection between the nearest point (inside) and the end (outside, unless it is inside too).
    let edge = |outside: f32| {
        if gap(outside) <= 0.0 {
            return outside;
        }
        let (mut inside, mut out) = (nearest, outside);
        for _ in 0..SEARCH_STEPS {
            let mid = (inside + out) / 2.0;
            if gap(mid) <= 0.0 {
                inside = mid;
            } else {
                out = mid;
            }
        }
        inside
    };
    Some((edge(0.0), edge(1.0)))
}

/// What is left of `path` once the intervals in `removed` (one sorted list per segment) are taken out, as pieces in
/// the order drawn; pieces shorter than [`MIN_PIECE`] are dropped.
fn pieces(path: &[Point], removed: &[Vec<(f32, f32)>]) -> Vec<Vec<Point>> {
    let mut pieces = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    for ((a, b), cuts) in segments(path).zip(removed) {
        let mut at = 0.0_f32;
        for &(start, end) in cuts {
            if start > at {
                push(&mut current, a.lerp(b, at));
                push(&mut current, a.lerp(b, start));
            }
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
            at = at.max(end);
        }
        if at < 1.0 {
            push(&mut current, a.lerp(b, at));
            push(&mut current, b);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces.retain(|piece| length(piece) >= MIN_PIECE);
    pieces
}

/// Adds `point` to `piece` unless it is where the piece already ends.
fn push(piece: &mut Vec<Point>, point: Point) {
    if piece.last() != Some(&point) {
        piece.push(point);
    }
}

/// The length of `path`.
fn length(path: &[Point]) -> f32 {
    path.windows(2)
        .map(|pair| pair[1].minus(pair[0]).length())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// A 100 pt stroke along y = 0.
    fn line() -> Vec<Point> {
        vec![p(0.0, 0.0), p(100.0, 0.0)]
    }

    fn target(path: &[Point], highlighter: bool) -> Target<'_> {
        Target {
            path,
            width: 2.0,
            highlighter,
        }
    }

    #[test]
    fn area_erasing_through_a_stroke_leaves_two_correct_strokes() {
        let path = line();
        // Straight down across the middle, radius 4: with the stroke's half width, 5 either side of x = 50.
        let pass = Pass::new(Eraser::Area, &[p(50.0, -20.0), p(50.0, 20.0)], 4.0);
        let Cut::Split(pieces) = pass.cut(target(&path, false)) else {
            panic!("the stroke is split");
        };
        assert_eq!(pieces.len(), 2, "two strokes: {pieces:?}");
        let close = |a: Point, b: Point| (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3;
        assert!(close(pieces[0][0], p(0.0, 0.0)), "{pieces:?}");
        assert!(
            close(*pieces[0].last().unwrap(), p(45.0, 0.0)),
            "{pieces:?}"
        );
        assert!(close(pieces[1][0], p(55.0, 0.0)), "{pieces:?}");
        assert!(
            close(*pieces[1].last().unwrap(), p(100.0, 0.0)),
            "{pieces:?}"
        );
    }

    #[test]
    fn a_stroke_eraser_removes_what_it_touches_and_nothing_else() {
        let path = line();
        let touching = Pass::new(Eraser::Stroke, &[p(30.0, 3.5)], 3.0);
        assert_eq!(
            touching.cut(target(&path, false)),
            Cut::Removed,
            "reaches the edge"
        );
        let missing = Pass::new(Eraser::Stroke, &[p(30.0, 10.0), p(60.0, 10.0)], 3.0);
        assert_eq!(missing.cut(target(&path, false)), Cut::Kept);
    }

    #[test]
    fn the_highlighter_eraser_leaves_pen_ink() {
        let path = line();
        let pass = Pass::new(Eraser::Highlighter, &[p(50.0, -5.0), p(50.0, 5.0)], 3.0);
        assert_eq!(pass.cut(target(&path, false)), Cut::Kept);
        assert_eq!(pass.cut(target(&path, true)), Cut::Removed);
    }

    #[test]
    fn erasing_an_end_shortens_the_stroke_and_erasing_all_removes_it() {
        let path = line();
        let end = Pass::new(Eraser::Area, &[p(100.0, -10.0), p(100.0, 10.0)], 9.0);
        let Cut::Split(pieces) = end.cut(target(&path, false)) else {
            panic!("shortened");
        };
        assert_eq!(pieces.len(), 1);
        assert!(
            (pieces[0].last().unwrap().x - 90.0).abs() < 1e-3,
            "{pieces:?}"
        );
        let all = Pass::new(Eraser::Area, &[p(-10.0, 0.0), p(110.0, 0.0)], 5.0);
        assert_eq!(all.cut(target(&path, false)), Cut::Removed);
    }

    #[test]
    fn crumbs_go_with_the_cut() {
        // Down at x = 40 and up at x = 50, reach 4.8 each: cuts to 44.8 and from 45.2 leave a 0.4 pt crumb.
        let path = line();
        let pass = Pass::new(
            Eraser::Area,
            &[p(40.0, -10.0), p(40.0, 10.0), p(50.0, 10.0), p(50.0, -10.0)],
            3.8,
        );
        let Cut::Split(pieces) = pass.cut(target(&path, false)) else {
            panic!("split");
        };
        assert!(
            pieces.iter().all(|piece| length(piece) >= MIN_PIECE),
            "{pieces:?}"
        );
        assert_eq!(pieces.len(), 2, "{pieces:?}");
    }

    #[test]
    fn a_segment_meets_a_capsule_in_one_exact_interval() {
        // Segment along y = 0; capsule a point at (5, 3), reach 5: x from 1 to 9 on a 0..10 segment.
        let (start, end) =
            interval(p(0.0, 0.0), p(10.0, 0.0), p(5.0, 3.0), p(5.0, 3.0), 5.0).unwrap();
        assert!(
            (start - 0.1).abs() < 1e-4 && (end - 0.9).abs() < 1e-4,
            "{start} {end}"
        );
        assert_eq!(
            interval(p(0.0, 0.0), p(10.0, 0.0), p(5.0, 9.0), p(5.0, 9.0), 5.0),
            None
        );
    }

    #[test]
    fn names_go_both_ways() {
        for eraser in Eraser::ALL {
            assert_eq!(Eraser::from_name(eraser.name()), Some(eraser));
        }
        assert_eq!(Eraser::from_name("sponge"), None);
    }
}
