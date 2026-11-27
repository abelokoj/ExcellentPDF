//! Handwriting beautification (plan H.19): smooths high-frequency pen jitter, evens out
//! letter heights, and regularizes intra-word horizontal spacing along each line while keeping
//! stroke endpoints anchored and preserving reversible snapshots.

use crate::erase::Point;
use crate::straighten::{LineSpacing, StraightenOptions, straighten};
use crate::stroke::InkPoint;

/// Default beautification strength (`0.5`).
pub const DEFAULT_STRENGTH: f32 = 0.5;

/// How letter heights and spacing are normalized during beautification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LetterNormalization {
    /// Smooth jitter only; keep original letter heights and horizontal gaps.
    SmoothOnly,
    /// Smooth jitter and gently normalize letter heights and intra-word spacing per line.
    #[default]
    HeightAndSpacing,
}

/// Parameters controlling handwriting beautification (plan H.19).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeautifyOptions {
    /// Strength in `0.0..=1.0` (`0.0` leaves strokes unchanged, `1.0` applies full smoothing and normalization).
    pub strength: f32,
    /// Whether to normalize letter heights and spacing in addition to smoothing pen jitter.
    pub normalization: LetterNormalization,
}

impl Default for BeautifyOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl BeautifyOptions {
    /// Default beautification options (`strength = 0.5`, height and spacing normalization enabled).
    #[must_use]
    pub fn new() -> Self {
        Self {
            strength: DEFAULT_STRENGTH,
            normalization: LetterNormalization::HeightAndSpacing,
        }
    }

    fn clamped_strength(self) -> f32 {
        if self.strength.is_finite() {
            self.strength.clamp(0.0, 1.0)
        } else {
            DEFAULT_STRENGTH
        }
    }
}

/// Reversible beautification output holding both the beautified and original strokes (plan H.19).
#[derive(Debug, Clone, PartialEq)]
pub struct BeautifyResult {
    /// Beautified strokes in the same order as input.
    pub strokes: Vec<Vec<InkPoint>>,
    /// Exact original strokes captured before beautifying, for one-step reversion.
    pub original: Vec<Vec<InkPoint>>,
}

impl BeautifyResult {
    /// Restores the original strokes untouched by beautification.
    #[must_use]
    pub fn revert(&self) -> Vec<Vec<InkPoint>> {
        self.original.clone()
    }
}

/// Beautifies a selection of handwritten strokes (plan H.19): smooths jitter, optionally evens out
/// letter size and horizontal spacing within each line, and returns a reversible [`BeautifyResult`].
#[must_use]
pub fn beautify_ink<S: AsRef<[InkPoint]>>(
    strokes: &[S],
    options: BeautifyOptions,
) -> BeautifyResult {
    let original: Vec<Vec<InkPoint>> = strokes.iter().map(|s| s.as_ref().to_vec()).collect();
    let strength = options.clamped_strength();
    if original.is_empty() || strength <= 0.0 {
        return BeautifyResult {
            strokes: original.clone(),
            original,
        };
    }

    // Step 1: Smooth high-frequency jitter along each stroke while keeping endpoints pinned.
    let mut smoothed: Vec<Vec<InkPoint>> = original
        .iter()
        .map(|stroke| smooth_jitter(stroke, strength))
        .collect();

    if options.normalization == LetterNormalization::HeightAndSpacing {
        normalize_lines(&mut smoothed, strength);
    }

    BeautifyResult {
        strokes: smoothed,
        original,
    }
}

fn smooth_jitter(stroke: &[InkPoint], strength: f32) -> Vec<InkPoint> {
    if stroke.len() < 3 {
        return stroke.to_vec();
    }
    let passes = if strength >= 0.7 { 2 } else { 1 };
    let alpha = (strength * 0.65).clamp(0.0, 0.7);
    let mut curr = stroke.to_vec();
    let mut next = curr.clone();

    for _ in 0..passes {
        for i in 1..curr.len() - 1 {
            let prev = curr[i - 1];
            let mid = curr[i];
            let fwd = curr[i + 1];
            let avg_x = 0.25 * prev.x + 0.5 * mid.x + 0.25 * fwd.x;
            let avg_y = 0.25 * prev.y + 0.5 * mid.y + 0.25 * fwd.y;
            next[i] = InkPoint {
                x: mid.x + (avg_x - mid.x) * alpha,
                y: mid.y + (avg_y - mid.y) * alpha,
                width: mid.width,
            };
        }
        curr.clone_from(&next);
    }
    curr
}

#[derive(Debug, Clone, Copy)]
struct BoxMetrics {
    index: usize,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl BoxMetrics {
    fn height(self) -> f32 {
        (self.max_y - self.min_y).max(0.5)
    }
}

fn stroke_box(index: usize, pts: &[InkPoint]) -> Option<BoxMetrics> {
    let first = pts.first()?;
    let (mut min_x, mut max_x) = (first.x, first.x);
    let (mut min_y, mut max_y) = (first.y, first.y);
    for p in pts.iter().skip(1) {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    Some(BoxMetrics {
        index,
        min_x,
        max_x,
        min_y,
        max_y,
    })
}

fn normalize_lines(strokes: &mut [Vec<InkPoint>], strength: f32) {
    let plain: Vec<Vec<Point>> = strokes
        .iter()
        .map(|s| s.iter().map(|p| Point { x: p.x, y: p.y }).collect())
        .collect();
    let straight = straighten(
        &plain,
        StraightenOptions {
            max_angle_degrees: 25.0,
            spacing: LineSpacing::KeepBaselines,
        },
    );

    for baseline in &straight.baselines {
        let mut items: Vec<BoxMetrics> = baseline
            .stroke_indices
            .iter()
            .filter_map(|&idx| stroke_box(idx, &strokes[idx]))
            .collect();
        if items.len() < 2 {
            continue;
        }
        items.sort_by(|a, b| a.min_x.total_cmp(&b.min_x));

        // Median height ignoring tiny punctuation/dots.
        let mut heights: Vec<f32> = items.iter().map(|m| m.height()).collect();
        heights.sort_by(f32::total_cmp);
        let median_h = heights[heights.len() / 2].max(1.0);
        let min_letter_h = median_h * 0.4;

        // 1. Gently normalize letter height around baseline anchor without distorting dots.
        let mut adjusted_boxes = Vec::with_capacity(items.len());
        for m in &items {
            let h = m.height();
            if h >= min_letter_h {
                let target_scale = (median_h / h).clamp(0.75, 1.35);
                let scale = 1.0 + (target_scale - 1.0) * (strength * 0.45);
                let anchor_y = m.max_y;
                let anchor_x = 0.5 * (m.min_x + m.max_x);
                for p in &mut strokes[m.index] {
                    p.y = anchor_y + (p.y - anchor_y) * scale;
                    p.x = anchor_x + (p.x - anchor_x) * scale;
                }
            }
            if let Some(updated) = stroke_box(m.index, &strokes[m.index]) {
                adjusted_boxes.push(updated);
            }
        }

        // 2. Group adjacent strokes into horizontal clusters (letters/sub-strokes) and even small gaps.
        if adjusted_boxes.len() < 3 {
            continue;
        }
        let mut positive_gaps: Vec<f32> = adjusted_boxes
            .windows(2)
            .map(|w| w[1].min_x - w[0].max_x)
            .filter(|&g| g > 0.0 && g < median_h * 1.1)
            .collect();
        if positive_gaps.is_empty() {
            continue;
        }
        positive_gaps.sort_by(f32::total_cmp);
        let target_gap = positive_gaps[positive_gaps.len() / 2];

        let mut cumulative_shift = 0.0_f32;
        for w in adjusted_boxes.windows(2) {
            let gap = w[1].min_x - w[0].max_x;
            if gap > 0.0 && gap < median_h * 1.1 {
                let desired_gap = gap + (target_gap - gap) * (strength * 0.5);
                cumulative_shift += desired_gap - gap;
            }
            if cumulative_shift.abs() > 1e-4 {
                for p in &mut strokes[w[1].index] {
                    p.x += cumulative_shift;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> InkPoint {
        InkPoint { x, y, width: 2.0 }
    }

    fn jitter_energy(pts: &[InkPoint]) -> f32 {
        pts.windows(3)
            .map(|w| {
                let mid_y = 0.5 * (w[0].y + w[2].y);
                (w[1].y - mid_y).abs()
            })
            .sum()
    }

    #[test]
    fn beautify_smooths_jitter_and_preserves_endpoints() {
        let jagged: Vec<InkPoint> = (0..11)
            .map(|i| p(i as f32 * 5.0, if i % 2 == 0 { 10.0 } else { 16.0 }))
            .collect();
        let res = beautify_ink(
            std::slice::from_ref(&jagged),
            BeautifyOptions {
                strength: 0.85,
                normalization: LetterNormalization::SmoothOnly,
            },
        );
        let before = jitter_energy(&jagged);
        let after = jitter_energy(&res.strokes[0]);
        assert!(after < before * 0.6, "before={before}, after={after}");
        assert_eq!(res.strokes[0].first(), jagged.first());
        assert_eq!(res.strokes[0].last(), jagged.last());
        assert_eq!(res.revert(), vec![jagged]);
    }

    #[test]
    fn beautify_evens_letter_heights_and_is_reversible() {
        // Three vertical strokes simulating letters of uneven height (18, 28, 20) along a line.
        let s1 = vec![p(10.0, 22.0), p(10.0, 40.0)];
        let s2 = vec![p(25.0, 12.0), p(25.0, 40.0)];
        let s3 = vec![p(48.0, 20.0), p(48.0, 40.0)];
        let res = beautify_ink(
            &[s1.clone(), s2.clone(), s3.clone()],
            BeautifyOptions::new(),
        );
        let span_y = |pts: &[InkPoint]| {
            let min = pts.iter().map(|pt| pt.y).fold(f32::INFINITY, f32::min);
            let max = pts.iter().map(|pt| pt.y).fold(f32::NEG_INFINITY, f32::max);
            max - min
        };
        let orig_spread = span_y(&s2) - span_y(&s1);
        let new_spread = span_y(&res.strokes[1]) - span_y(&res.strokes[0]);
        assert!(
            new_spread < orig_spread,
            "orig={orig_spread}, new={new_spread}"
        );
        assert_eq!(res.revert(), vec![s1, s2, s3]);
    }
}
