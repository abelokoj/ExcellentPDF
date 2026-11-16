//! A stroke as the pen draws it: points in page points, each with its own width (plan §8.3: per-point widths, not
//! one width for the stroke), and the samples they came from, for saving (H.9), replay and recording (§16).

use crate::sample::Sample;
use crate::smooth::{catmull_rom, filter, flatten};
use crate::width::WidthCurve;

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

/// The shortest time between two samples speed is measured over, in milliseconds: samples a device stamps with the
/// same time don't make the pen infinitely fast.
const MIN_INTERVAL_MS: f64 = 1.0;

/// The points of one stroke, in the order drawn, and the samples behind them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stroke {
    points: Vec<InkPoint>,
    samples: Vec<Sample>,
}

impl Stroke {
    /// An empty stroke.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The stroke `samples` (in page points, oldest first) draw with `curve`.
    #[must_use]
    pub fn from_samples(samples: &[Sample], curve: &WidthCurve) -> Self {
        let mut stroke = Self::new();
        for &sample in samples {
            stroke.push_sample(sample, curve);
        }
        stroke
    }

    /// Adds `sample` (in page points), its width from `curve` at the speed since the last one.
    pub fn push_sample(&mut self, sample: Sample, curve: &WidthCurve) {
        let speed = self.samples.last().map_or(0.0, |last| {
            let elapsed = (sample.time - last.time).max(MIN_INTERVAL_MS);
            sample.distance(last) / elapsed as f32
        });
        self.samples.push(sample);
        self.push(InkPoint {
            x: sample.x,
            y: sample.y,
            width: curve.width(sample.pressure, sample.tilt, speed),
        });
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

    /// The samples behind the points, in the order drawn; empty for a stroke built from points.
    #[must_use]
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The stroke smoothed (plan §8.2, step 4): filtered by `steadiness` (0 none, towards 1 heavy), through a
    /// Catmull-Rom spline, and flattened back to points within `tolerance` page points of it.
    #[must_use]
    pub fn smoothed(&self, steadiness: f32, tolerance: f32) -> Vec<InkPoint> {
        if self.points.len() < 3 {
            return self.points.clone();
        }
        flatten(&catmull_rom(&filter(&self.points, steadiness)), tolerance)
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
    fn a_pen_held_still_widens_the_last_point() {
        let mut stroke = Stroke::new();
        stroke.push(at(1.0, 1.0, 1.0));
        stroke.push(at(1.0, 1.0, 2.0));
        stroke.push(at(1.0, 1.0, 1.5));
        stroke.push(at(2.0, 1.0, 1.0));
        assert_eq!(stroke.points(), &[at(1.0, 1.0, 2.0), at(2.0, 1.0, 1.0)]);
    }

    #[test]
    fn a_fast_hand_draws_thinner() {
        let curve = WidthCurve {
            speed: 1.0,
            ..WidthCurve::linear(2.0, 2.0)
        };
        let slow = Stroke::from_samples(
            &[Sample::at(0.0, 0.0, 0.0), Sample::at(1.0, 0.0, 10.0)],
            &curve,
        );
        let fast = Stroke::from_samples(
            &[Sample::at(0.0, 0.0, 0.0), Sample::at(10.0, 0.0, 1.0)],
            &curve,
        );
        assert_eq!(slow.points()[0].width, 2.0, "the first point has no speed");
        assert!(fast.points()[1].width < slow.points()[1].width);
        assert_eq!(fast.samples().len(), 2);
    }

    #[test]
    fn smoothing_keeps_the_ends_and_adds_curve_points() {
        let samples: Vec<Sample> = (0..6)
            .map(|i| {
                Sample::at(
                    i as f32 * 10.0,
                    if i % 2 == 0 { 0.0 } else { 10.0 },
                    f64::from(i) * 8.0,
                )
            })
            .collect();
        let stroke = Stroke::from_samples(&samples, &WidthCurve::default());
        let smooth = stroke.smoothed(0.0, 0.1);
        assert!(smooth.len() > stroke.len());
        assert_eq!(smooth.first(), stroke.points().first());
        assert_eq!(smooth.last(), stroke.points().last());
    }
}
