//! Prediction (plan §8.2, step 3): where the pen will be a frame or two from now, so the preview keeps up with the
//! hand. Where the OS predicts (iOS, Android's motion predictor) its points are used instead; elsewhere this
//! extrapolates at the pen's recent velocity. A predicted point is drawn once and replaced by the real samples.

use crate::sample::Sample;

/// How far back the velocity is measured, in milliseconds: long enough to steady it, short enough to follow turns.
const WINDOW_MS: f64 = 24.0;

/// Where the pen will be `ahead` milliseconds after the newest of `samples` (oldest first), moving at its velocity
/// over the last few samples, and at most `max_travel` from where it is. `None` with fewer than two samples in the
/// window, or when the pen stands still.
#[must_use]
pub fn predict(samples: &[Sample], ahead: f64, max_travel: f32) -> Option<(f32, f32)> {
    let newest = samples.last()?;
    let oldest = samples
        .iter()
        .rev()
        .take_while(|sample| newest.time - sample.time <= WINDOW_MS)
        .last()?;
    let elapsed = newest.time - oldest.time;
    if elapsed <= 0.0 {
        return None;
    }
    let scale = (ahead / elapsed) as f32;
    let (dx, dy) = ((newest.x - oldest.x) * scale, (newest.y - oldest.y) * scale);
    let travel = dx.hypot(dy);
    if travel == 0.0 {
        return None;
    }
    let keep = (max_travel / travel).min(1.0);
    Some((newest.x + dx * keep, newest.y + dy * keep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_pen_is_followed_ahead() {
        let samples: Vec<Sample> = (0..5)
            .map(|i| Sample::at(i as f32 * 2.0, 10.0, f64::from(i) * 4.0))
            .collect();
        // 2 units every 4 ms: 8 ms ahead is 4 units on.
        assert_eq!(predict(&samples, 8.0, 100.0), Some((12.0, 10.0)));
        // Never further than asked.
        assert_eq!(predict(&samples, 8.0, 1.0), Some((9.0, 10.0)));
    }

    #[test]
    fn no_prediction_without_movement_or_history() {
        assert_eq!(predict(&[], 8.0, 10.0), None);
        assert_eq!(predict(&[Sample::at(1.0, 1.0, 0.0)], 8.0, 10.0), None);
        let still = [Sample::at(1.0, 1.0, 0.0), Sample::at(1.0, 1.0, 4.0)];
        assert_eq!(predict(&still, 8.0, 10.0), None);
        // A sample older than the window doesn't count.
        let stale = [Sample::at(0.0, 0.0, 0.0), Sample::at(5.0, 0.0, 100.0)];
        assert_eq!(predict(&stale, 8.0, 10.0), None);
    }
}
