//! A stroke as the pen draws it: points in page points, each with its own width (plan §8.3: per-point widths, not
//! one width for the stroke).

/// One point of a stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InkPoint {
    /// Across, in page points.
    pub x: f32,
    /// Down, in page points.
    pub y: f32,
    /// The stroke's width here, in page points.
    pub width: f32,
}

/// How a pen's width follows pressure.
// shortcut: linear from `min` to `max`; H.1 adds tilt, speed and per-pen response curves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthCurve {
    /// Width at no pressure, in page points.
    pub min: f32,
    /// Width at full pressure, in page points.
    pub max: f32,
}

impl WidthCurve {
    /// The width at `pressure` (0 to 1); a pen that reports no pressure draws halfway.
    #[must_use]
    pub fn width(&self, pressure: Option<f32>) -> f32 {
        let pressure = pressure.unwrap_or(0.5).clamp(0.0, 1.0);
        self.min + (self.max - self.min) * pressure
    }
}

impl Default for WidthCurve {
    /// A fine pen: 0.4 to 2.4 points.
    fn default() -> Self {
        Self { min: 0.4, max: 2.4 }
    }
}

/// The points of one stroke, in the order drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stroke {
    points: Vec<InkPoint>,
}

impl Stroke {
    /// An empty stroke.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `point`. A point where the last one is only widens it, so a pen held still doesn't pile up points.
    pub fn push(&mut self, point: InkPoint) {
        if let Some(last) = self.points.last_mut()
            && last.x == point.x
            && last.y == point.y
        {
            last.width = last.width.max(point.width);
            return;
        }
        self.points.push(point);
    }

    /// The points, in the order drawn.
    #[must_use]
    pub fn points(&self) -> &[InkPoint] {
        &self.points
    }

    /// How many points.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether it has no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, width: f32) -> InkPoint {
        InkPoint { x, y, width }
    }

    #[test]
    fn width_follows_pressure_within_the_curve() {
        let curve = WidthCurve { min: 1.0, max: 3.0 };
        assert_eq!(curve.width(Some(0.0)), 1.0);
        assert_eq!(curve.width(Some(1.0)), 3.0);
        assert_eq!(curve.width(Some(0.25)), 1.5);
        assert_eq!(curve.width(None), 2.0);
        assert_eq!(curve.width(Some(7.0)), 3.0);
        assert_eq!(curve.width(Some(-1.0)), 1.0);
    }

    #[test]
    fn a_pen_held_still_widens_the_last_point() {
        let mut stroke = Stroke::new();
        stroke.push(at(1.0, 1.0, 1.0));
        stroke.push(at(1.0, 1.0, 2.0));
        stroke.push(at(1.0, 1.0, 1.5));
        stroke.push(at(2.0, 1.0, 1.0));
        assert_eq!(stroke.points(), &[at(1.0, 1.0, 2.0), at(2.0, 1.0, 1.0)]);
    }
}
