//! The lasso (H.6, plan §8.4): a freeform loop drawn around strokes and annotations selects those it encloses, and
//! the selection turns about its centre. Ported from SumatraPDF Enhanced's `LassoContains` and `RotateLassoPoint`
//! (`src/Canvas.cpp`); Enhanced has no lasso tests to port, so these are new.

use crate::erase::Point;

/// The fewest points a loop needs to enclose anything.
const MIN_POINTS: usize = 3;

/// A freeform loop, in page points, closed from its last point back to its first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lasso {
    path: Vec<Point>,
}

impl Lasso {
    /// The loop through `path`, in the order drawn.
    #[must_use]
    pub fn new(path: Vec<Point>) -> Self {
        Self { path }
    }

    /// Whether it can enclose anything: three points or more.
    #[must_use]
    pub fn is_loop(&self) -> bool {
        self.path.len() >= MIN_POINTS
    }

    /// Whether `point` is inside, by the even-odd rule (a loop that crosses itself leaves its twice-wound parts out).
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        if !self.is_loop() {
            return false;
        }
        let mut inside = false;
        let mut j = self.path.len() - 1;
        for (i, &a) in self.path.iter().enumerate() {
            let b = self.path[j];
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Whether every one of `points` is inside; none is not enclosed.
    #[must_use]
    pub fn encloses(&self, points: impl IntoIterator<Item = Point>) -> bool {
        let mut any = false;
        for point in points {
            if !self.contains(point) {
                return false;
            }
            any = true;
        }
        any
    }
}

/// `point` turned `degrees` clockwise on the page (y grows down) about `centre`.
#[must_use]
pub fn rotate(point: Point, centre: Point, degrees: f32) -> Point {
    let (sine, cosine) = degrees.to_radians().sin_cos();
    let (dx, dy) = (point.x - centre.x, point.y - centre.y);
    Point {
        x: centre.x + dx * cosine - dy * sine,
        y: centre.y + dx * sine + dy * cosine,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn square() -> Lasso {
        Lasso::new(vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)])
    }

    #[test]
    fn a_loop_holds_what_is_inside_it() {
        let lasso = square();
        assert!(lasso.contains(p(5.0, 5.0)));
        assert!(!lasso.contains(p(15.0, 5.0)));
        assert!(!lasso.contains(p(5.0, -1.0)));
        assert!(lasso.encloses([p(1.0, 1.0), p(9.0, 9.0)]));
        assert!(
            !lasso.encloses([p(1.0, 1.0), p(11.0, 9.0)]),
            "one point out"
        );
        assert!(!lasso.encloses([]), "nothing is not enclosed");
    }

    #[test]
    fn a_concave_loop_leaves_its_notch_out() {
        // A U shape: the gap between its arms is outside.
        let u = Lasso::new(vec![
            p(0.0, 0.0),
            p(3.0, 0.0),
            p(3.0, 7.0),
            p(7.0, 7.0),
            p(7.0, 0.0),
            p(10.0, 0.0),
            p(10.0, 10.0),
            p(0.0, 10.0),
        ]);
        assert!(u.contains(p(1.0, 1.0)), "an arm");
        assert!(!u.contains(p(5.0, 3.0)), "the notch");
        assert!(u.contains(p(5.0, 9.0)), "the base");
    }

    #[test]
    fn two_points_enclose_nothing() {
        let line = Lasso::new(vec![p(0.0, 0.0), p(10.0, 10.0)]);
        assert!(!line.is_loop());
        assert!(!line.contains(p(5.0, 5.0)));
    }

    #[test]
    fn turning_goes_clockwise_on_the_page_and_back() {
        let turned = rotate(p(10.0, 0.0), p(0.0, 0.0), 90.0);
        assert!(
            (turned.x - 0.0).abs() < 1e-5 && (turned.y - 10.0).abs() < 1e-5,
            "{turned:?}"
        );
        let back = rotate(turned, p(0.0, 0.0), -90.0);
        assert!(
            (back.x - 10.0).abs() < 1e-5 && back.y.abs() < 1e-5,
            "{back:?}"
        );
        assert_eq!(
            rotate(p(3.0, 4.0), p(3.0, 4.0), 37.0),
            p(3.0, 4.0),
            "the centre stays"
        );
    }
}
