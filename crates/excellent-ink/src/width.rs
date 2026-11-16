//! A pen's width from pressure, tilt and speed (plan §8.2, step 5): w = min + (max − min) × p^exponent, widened as
//! the pen leans over and thinned as it moves fast.

use crate::sample::Tilt;

/// How a pen's width follows the hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthCurve {
    /// Width at no pressure, in page points.
    pub min: f32,
    /// Width at full pressure, in page points.
    pub max: f32,
    /// The pressure response: 1 is linear, above 1 needs a firmer hand for the same width.
    pub exponent: f32,
    /// How much the pen lying flat widens the line: 0 not at all, 1 doubles it (a pencil's shading).
    pub tilt: f32,
    /// How much speed thins the line: the width is divided by 1 + `speed` × points per millisecond.
    pub speed: f32,
}

/// What a pen does when the device reports no pressure: draw halfway.
const NO_PRESSURE: f32 = 0.5;

impl WidthCurve {
    /// A curve from `min` to `max` that follows pressure in a straight line and ignores tilt and speed.
    #[must_use]
    pub fn linear(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            exponent: 1.0,
            tilt: 0.0,
            speed: 0.0,
        }
    }

    /// The width at `pressure` (0 to 1; `None` draws halfway), leaning as `tilt` says, moving at `speed` page points
    /// per millisecond.
    #[must_use]
    pub fn width(&self, pressure: Option<f32>, tilt: Option<Tilt>, speed: f32) -> f32 {
        let pressure = pressure.unwrap_or(NO_PRESSURE).clamp(0.0, 1.0);
        let mut width = self.min + (self.max - self.min) * pressure.powf(self.exponent);
        if let Some(tilt) = tilt {
            let flat = (1.0 - tilt.altitude() / 90.0).clamp(0.0, 1.0);
            width *= 1.0 + self.tilt * flat;
        }
        width / (1.0 + self.speed * speed.max(0.0))
    }
}

impl Default for WidthCurve {
    /// A fine pen: 0.4 to 2.4 points, the plan's 1.25 response, a little thinner when fast.
    fn default() -> Self {
        Self {
            min: 0.4,
            max: 2.4,
            exponent: 1.25,
            tilt: 0.0,
            speed: 0.15,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_follows_pressure_within_the_curve() {
        let curve = WidthCurve::linear(1.0, 3.0);
        assert_eq!(curve.width(Some(0.0), None, 0.0), 1.0);
        assert_eq!(curve.width(Some(1.0), None, 0.0), 3.0);
        assert_eq!(curve.width(Some(0.25), None, 0.0), 1.5);
        assert_eq!(curve.width(None, None, 0.0), 2.0);
        assert_eq!(curve.width(Some(7.0), None, 0.0), 3.0);
        assert_eq!(curve.width(Some(-1.0), None, 0.0), 1.0);
    }

    #[test]
    fn the_response_curve_needs_a_firmer_hand() {
        let curve = WidthCurve {
            exponent: 1.25,
            ..WidthCurve::linear(0.0, 1.0)
        };
        let half = curve.width(Some(0.5), None, 0.0);
        assert!((half - 0.5_f32.powf(1.25)).abs() < 1e-6);
        assert!(half < 0.5);
    }

    #[test]
    fn leaning_widens_and_speed_thins() {
        let curve = WidthCurve {
            tilt: 1.0,
            speed: 1.0,
            ..WidthCurve::linear(2.0, 2.0)
        };
        let upright = Tilt { x: 0.0, y: 0.0 };
        assert_eq!(curve.width(None, Some(upright), 0.0), 2.0);
        let leaning = Tilt { x: 45.0, y: 0.0 };
        assert!((curve.width(None, Some(leaning), 0.0) - 3.0).abs() < 1e-4);
        assert_eq!(curve.width(None, None, 1.0), 1.0);
        assert_eq!(curve.width(None, None, -5.0), 2.0);
    }
}
