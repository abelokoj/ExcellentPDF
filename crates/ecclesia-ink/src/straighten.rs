//! Handwriting straightening (H.13, plan §27.7): finds each line's baseline in a selection of handwritten strokes
//! and applies a rigid Euclidean transform (rotation and translation) to make each line level while preserving exact
//! letter shapes.
//!
//! Coordinates are in page points (`x` right, `y` down). Multi-line sloped paragraphs are separated by first
//! estimating the dominant slope from local right-hand neighbor strokes, grouping strokes into lines perpendicular to
//! that slope, attaching small diacritics (such as `i`-dots) to their nearest line, and fitting each line's baseline
//! with iteratively reweighted least squares (IRLS) so descenders do not pull the baseline down.

use crate::erase::Point;
use crate::stroke::InkPoint;

/// Default maximum slope angle corrected by straightening, in degrees.
pub const DEFAULT_MAX_ANGLE_DEGREES: f32 = 45.0;

/// Minimum height ratio relative to median stroke height below which a stroke is treated as a diacritic or punctuation
/// mark (for example an `i`-dot or period) and attached to its nearest line rather than forming a line of its own.
const DIACRITIC_HEIGHT_RATIO: f32 = 0.35;

/// Vertical gap multiplier (times median stroke height) that separates two distinct lines of handwriting.
const LINE_GAP_FACTOR: f32 = 1.15;

/// Maximum horizontal neighbor search distance (times median stroke height) when estimating the global slope.
const NEIGHBOR_HORIZ_FACTOR: f32 = 4.5;

/// Number of Huber IRLS refinement passes after ordinary least squares so descenders do not bias the baseline.
const IRLS_ITERATIONS: usize = 3;

/// How vertical baseline spacing between consecutive lines is handled after each line is leveled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineSpacing {
    /// Keep each line anchored at its own baseline pivot height.
    #[default]
    KeepBaselines,
    /// Normalize gaps between consecutive lines to the median line spacing, anchored at the top line.
    EvenSpacing,
}

/// Parameters controlling handwriting straightening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightenOptions {
    /// Maximum slope magnitude corrected, in degrees (clamped to `0.0..=60.0`).
    pub max_angle_degrees: f32,
    /// Whether to preserve each line's vertical position or even out spacing between lines.
    pub spacing: LineSpacing,
}

impl Default for StraightenOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl StraightenOptions {
    /// Default options: up to 45° correction, keeping each line's baseline anchor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_angle_degrees: DEFAULT_MAX_ANGLE_DEGREES,
            spacing: LineSpacing::KeepBaselines,
        }
    }

    fn max_angle_radians(self) -> f32 {
        let deg = if self.max_angle_degrees.is_finite() {
            self.max_angle_degrees.clamp(0.0, 60.0)
        } else {
            DEFAULT_MAX_ANGLE_DEGREES
        };
        deg.to_radians()
    }
}

/// The detected baseline of one line of handwriting.
#[derive(Debug, Clone, PartialEq)]
pub struct Baseline {
    /// Anchor point on the fitted baseline (leftmost horizontal extent of the line).
    pub origin: Point,
    /// Fitted baseline angle in radians (positive slopes downward left-to-right in page y-down space).
    pub angle_radians: f32,
    /// Indices of the input strokes belonging to this line.
    pub stroke_indices: Vec<usize>,
}

/// The straightened strokes and the baselines that were detected and leveled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StraightenResult {
    /// Straightened strokes in the same order as the input strokes.
    pub strokes: Vec<Vec<Point>>,
    /// Detected baselines, ordered top to bottom.
    pub baselines: Vec<Baseline>,
}

/// Rigid Euclidean transform: rotates `point` by `angle_radians` around `pivot`, then shifts by `shift`.
///
/// Preserves all pairwise distances and angles exactly.
#[must_use]
pub fn rigid_transform(point: Point, pivot: Point, angle_radians: f32, shift: Point) -> Point {
    let (sin, cos) = angle_radians.sin_cos();
    let dx = point.x - pivot.x;
    let dy = point.y - pivot.y;
    Point {
        x: pivot.x + dx * cos - dy * sin + shift.x,
        y: pivot.y + dx * sin + dy * cos + shift.y,
    }
}

/// Straightens a selection of handwritten strokes, leveling each line's baseline while keeping exact letter shapes.
#[must_use]
pub fn straighten<S: AsRef<[Point]>>(
    strokes: &[S],
    options: StraightenOptions,
) -> StraightenResult {
    let point_strokes: Vec<&[Point]> = strokes.iter().map(AsRef::as_ref).collect();
    let (transforms, baselines) = compute_transforms(&point_strokes, options);
    let out_strokes = point_strokes
        .iter()
        .enumerate()
        .map(|(idx, stroke)| {
            let tf = transforms[idx];
            stroke
                .iter()
                .map(|&p| rigid_transform(p, tf.pivot, tf.angle, tf.shift))
                .collect()
        })
        .collect();
    StraightenResult {
        strokes: out_strokes,
        baselines,
    }
}

/// Straightens handwritten strokes that carry per-point widths, preserving every point's width unchanged.
#[must_use]
pub fn straighten_ink<S: AsRef<[InkPoint]>>(
    strokes: &[S],
    options: StraightenOptions,
) -> (Vec<Vec<InkPoint>>, Vec<Baseline>) {
    let plain: Vec<Vec<Point>> = strokes
        .iter()
        .map(|s| {
            s.as_ref()
                .iter()
                .map(|p| Point { x: p.x, y: p.y })
                .collect()
        })
        .collect();
    let refs: Vec<&[Point]> = plain.iter().map(Vec::as_slice).collect();
    let (transforms, baselines) = compute_transforms(&refs, options);
    let out = strokes
        .iter()
        .enumerate()
        .map(|(idx, stroke)| {
            let tf = transforms[idx];
            stroke
                .as_ref()
                .iter()
                .map(|p| {
                    let moved =
                        rigid_transform(Point { x: p.x, y: p.y }, tf.pivot, tf.angle, tf.shift);
                    InkPoint {
                        x: moved.x,
                        y: moved.y,
                        width: p.width,
                    }
                })
                .collect()
        })
        .collect();
    (out, baselines)
}

#[derive(Debug, Clone, Copy)]
struct StrokeMetrics {
    index: usize,
    centroid: Point,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    height: f32,
    width: f32,
}

const ZERO_POINT: Point = Point { x: 0.0, y: 0.0 };

#[derive(Debug, Clone, Copy)]
struct RigidTransform {
    pivot: Point,
    angle: f32,
    shift: Point,
}

impl RigidTransform {
    const fn identity() -> Self {
        Self {
            pivot: ZERO_POINT,
            angle: 0.0,
            shift: ZERO_POINT,
        }
    }
}

fn compute_transforms(
    strokes: &[&[Point]],
    options: StraightenOptions,
) -> (Vec<RigidTransform>, Vec<Baseline>) {
    let n = strokes.len();
    let mut transforms = vec![RigidTransform::identity(); n];
    if n == 0 {
        return (transforms, Vec::new());
    }

    let metrics: Vec<StrokeMetrics> = strokes
        .iter()
        .enumerate()
        .filter_map(|(index, pts)| stroke_metrics(index, pts))
        .collect();
    if metrics.is_empty() {
        return (transforms, Vec::new());
    }

    let max_angle = options.max_angle_radians();
    let median_h = median_height(&metrics);
    let is_diacritic = |m: &StrokeMetrics| {
        median_h > f32::EPSILON
            && m.height < median_h * DIACRITIC_HEIGHT_RATIO
            && m.width < median_h * DIACRITIC_HEIGHT_RATIO
    };

    let body: Vec<&StrokeMetrics> = metrics.iter().filter(|m| !is_diacritic(m)).collect();
    let body_or_all: Vec<&StrokeMetrics> = if body.is_empty() {
        metrics.iter().collect()
    } else {
        body
    };

    // Stage 1: Estimate dominant paragraph slope from right-hand neighbors so tilted lines separate cleanly.
    let global_angle = estimate_global_slope(&body_or_all, median_h, max_angle);

    // Stage 2: Group body strokes into lines along the axis perpendicular to `global_angle`.
    let mut lines = group_into_lines(&body_or_all, global_angle, median_h);

    // Attach diacritics / punctuation to the line containing their nearest body stroke.
    for m in metrics.iter().filter(|m| is_diacritic(m)) {
        if let Some(best_line) = nearest_line(m, &lines, strokes) {
            lines[best_line].push(m.index);
        } else if let Some(first) = lines.first_mut() {
            first.push(m.index);
        }
    }

    // Stage 3: Fit each line's baseline via robust IRLS on stroke bottoms.
    let mut baselines: Vec<Baseline> = Vec::with_capacity(lines.len());
    for mut line_indices in lines {
        line_indices.sort_unstable();
        let line_body: Vec<usize> = line_indices
            .iter()
            .copied()
            .filter(|&idx| {
                metrics
                    .iter()
                    .find(|m| m.index == idx)
                    .is_some_and(|m| !is_diacritic(m))
            })
            .collect();
        let fit_indices = if line_body.is_empty() {
            &line_indices
        } else {
            &line_body
        };

        let bottom_pts: Vec<Point> = fit_indices
            .iter()
            .filter_map(|&idx| stroke_bottom_point(strokes[idx], global_angle))
            .collect();

        let (origin, raw_angle) = fit_baseline_irls(&bottom_pts, global_angle);
        let angle_radians = raw_angle.clamp(-max_angle, max_angle);
        baselines.push(Baseline {
            origin,
            angle_radians,
            stroke_indices: line_indices,
        });
    }

    // Sort baselines top-to-bottom by their origin's y coordinate.
    baselines.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));

    // Optional even line spacing: compute vertical shift per line.
    let shifts: Vec<f32> = match options.spacing {
        LineSpacing::KeepBaselines => vec![0.0; baselines.len()],
        LineSpacing::EvenSpacing => {
            if baselines.len() <= 2 {
                vec![0.0; baselines.len()]
            } else {
                let mut gaps: Vec<f32> = baselines
                    .windows(2)
                    .map(|pair| pair[1].origin.y - pair[0].origin.y)
                    .collect();
                gaps.sort_by(f32::total_cmp);
                let step = gaps[gaps.len() / 2];
                let top_y = baselines[0].origin.y;
                baselines
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (top_y + step * i as f32) - b.origin.y)
                    .collect()
            }
        }
    };

    for (baseline, dy) in baselines.iter().zip(shifts) {
        let tf = RigidTransform {
            pivot: baseline.origin,
            angle: -baseline.angle_radians,
            shift: Point { x: 0.0, y: dy },
        };
        for &idx in &baseline.stroke_indices {
            transforms[idx] = tf;
        }
    }

    (transforms, baselines)
}

fn stroke_metrics(index: usize, pts: &[Point]) -> Option<StrokeMetrics> {
    if pts.is_empty() {
        return None;
    }
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut sx, mut sy) = (0.0_f64, 0.0_f64);
    for &p in pts {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
        sx += f64::from(p.x);
        sy += f64::from(p.y);
    }
    let count = pts.len() as f64;
    Some(StrokeMetrics {
        index,
        centroid: Point {
            x: (sx / count) as f32,
            y: (sy / count) as f32,
        },
        min_x,
        max_x,
        min_y,
        max_y,
        height: (max_y - min_y).max(0.0),
        width: (max_x - min_x).max(0.0),
    })
}

fn median_height(metrics: &[StrokeMetrics]) -> f32 {
    let mut heights: Vec<f32> = metrics
        .iter()
        .map(|m| m.height.max(m.width * 0.25))
        .filter(|&h| h > f32::EPSILON)
        .collect();
    if heights.is_empty() {
        return 10.0;
    }
    heights.sort_by(f32::total_cmp);
    heights[heights.len() / 2]
}

fn estimate_global_slope(body: &[&StrokeMetrics], median_h: f32, max_angle: f32) -> f32 {
    if body.len() < 2 {
        return 0.0;
    }
    let max_dist = (median_h * NEIGHBOR_HORIZ_FACTOR).max(20.0);
    let mut angles: Vec<f32> = Vec::new();
    for &a in body {
        let mut best: Option<(f32, f32)> = None;
        for &b in body {
            if a.index == b.index {
                continue;
            }
            let dx = b.centroid.x - a.centroid.x;
            let dy = b.centroid.y - a.centroid.y;
            if dx <= median_h * 0.15 {
                continue;
            }
            let dist = dx.hypot(dy);
            if dist > max_dist {
                continue;
            }
            let angle = dy.atan2(dx);
            if angle.abs() > max_angle {
                continue;
            }
            if best.is_none_or(|(best_d, _)| dist < best_d) {
                best = Some((dist, angle));
            }
        }
        if let Some((_, angle)) = best {
            angles.push(angle);
        }
    }
    if angles.is_empty() {
        // Fallback: simple OLS over centroids when strokes are widely spaced.
        let pts: Vec<Point> = body.iter().map(|m| m.centroid).collect();
        let (_, angle) = fit_ols(&pts);
        return angle.clamp(-max_angle, max_angle);
    }
    angles.sort_by(f32::total_cmp);
    angles[angles.len() / 2].clamp(-max_angle, max_angle)
}

/// Groups body strokes into lines using projections perpendicular to `global_angle`.
fn group_into_lines(body: &[&StrokeMetrics], global_angle: f32, median_h: f32) -> Vec<Vec<usize>> {
    let (sin, cos) = global_angle.sin_cos();
    let perp = |p: Point| -p.x * sin + p.y * cos;
    let gap_threshold = (median_h * LINE_GAP_FACTOR).max(6.0);

    let mut ordered: Vec<(f32, &StrokeMetrics)> =
        body.iter().map(|&m| (perp(m.centroid), m)).collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut groups: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for (v, m) in ordered {
        // Vertical extent of the stroke along the perpendicular axis.
        let corners = [
            Point {
                x: m.min_x,
                y: m.min_y,
            },
            Point {
                x: m.max_x,
                y: m.min_y,
            },
            Point {
                x: m.min_x,
                y: m.max_y,
            },
            Point {
                x: m.max_x,
                y: m.max_y,
            },
        ];
        let v_min = corners
            .iter()
            .map(|&c| perp(c))
            .fold(f32::INFINITY, f32::min);
        let v_max = corners
            .iter()
            .map(|&c| perp(c))
            .fold(f32::NEG_INFINITY, f32::max);

        if let Some((mean_v, max_v, indices)) = groups.last_mut()
            && (v - *mean_v).abs() <= gap_threshold
            && (v_min <= *max_v + median_h * 0.35)
        {
            let count = indices.len() as f32;
            *mean_v = (*mean_v * count + v) / (count + 1.0);
            *max_v = max_v.max(v_max);
            indices.push(m.index);
            continue;
        }
        groups.push((v, v_max, vec![m.index]));
    }

    groups.into_iter().map(|(_, _, indices)| indices).collect()
}

fn nearest_line(m: &StrokeMetrics, lines: &[Vec<usize>], strokes: &[&[Point]]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (line_idx, indices) in lines.iter().enumerate() {
        for &stroke_idx in indices {
            for &p in strokes[stroke_idx] {
                let d = (p.x - m.centroid.x).hypot(p.y - m.centroid.y);
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((line_idx, d));
                }
            }
        }
    }
    best.map(|(idx, _)| idx)
}

/// Bottom-most anchor of a stroke along the perpendicular to `approx_angle`.
fn stroke_bottom_point(pts: &[Point], approx_angle: f32) -> Option<Point> {
    if pts.is_empty() {
        return None;
    }
    let (sin, cos) = approx_angle.sin_cos();
    let perp = |p: Point| -p.x * sin + p.y * cos;
    let max_v = pts
        .iter()
        .map(|&p| perp(p))
        .fold(f32::NEG_INFINITY, f32::max);
    let min_v = pts.iter().map(|&p| perp(p)).fold(f32::INFINITY, f32::min);
    let band = ((max_v - min_v) * 0.05).max(0.5);
    let (mut sx, mut sy, mut count) = (0.0_f64, 0.0_f64, 0_usize);
    for &p in pts {
        if max_v - perp(p) <= band {
            sx += f64::from(p.x);
            sy += f64::from(p.y);
            count += 1;
        }
    }
    if count == 0 {
        return pts.first().copied();
    }
    Some(Point {
        x: (sx / count as f64) as f32,
        y: (sy / count as f64) as f32,
    })
}

/// Ordinary least squares line fit `y = a + b * x`, returning `(origin_at_min_x, atan(b))`.
fn fit_ols(points: &[Point]) -> (Point, f32) {
    fit_weighted_ls(points, &vec![1.0_f64; points.len()])
}

/// Iteratively reweighted least squares (Huber weights, down-weighting positive residuals that come from descenders).
fn fit_baseline_irls(points: &[Point], fallback_angle: f32) -> (Point, f32) {
    if points.is_empty() {
        return (ZERO_POINT, 0.0);
    }
    if points.len() == 1 {
        return (points[0], fallback_angle);
    }
    let x_span = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max)
        - points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    if x_span < 1.0 {
        return (points[0], fallback_angle);
    }

    let mut weights = vec![1.0_f64; points.len()];
    let mut fit = fit_weighted_ls(points, &weights);

    for _ in 0..IRLS_ITERATIONS {
        let (origin, angle) = fit;
        let slope = f64::from(angle.tan());
        let residuals: Vec<f64> = points
            .iter()
            .map(|p| {
                let pred = f64::from(origin.y) + slope * f64::from(p.x - origin.x);
                f64::from(p.y) - pred
            })
            .collect();
        let mut abs_res: Vec<f64> = residuals.iter().map(|r| r.abs()).collect();
        abs_res.sort_by(f64::total_cmp);
        let mad = (abs_res[abs_res.len() / 2] * 1.4826).max(1.5);
        for (w, &r) in weights.iter_mut().zip(&residuals) {
            // Down-weight outliers, and extra down-weight points far below the line (descenders like g/p/y).
            let scale = if r > 0.0 { mad * 0.8 } else { mad * 1.345 };
            let abs_r = r.abs();
            *w = if abs_r <= scale { 1.0 } else { scale / abs_r };
        }
        fit = fit_weighted_ls(points, &weights);
    }
    fit
}

fn fit_weighted_ls(points: &[Point], weights: &[f64]) -> (Point, f32) {
    if points.is_empty() {
        return (ZERO_POINT, 0.0);
    }
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let (mut sw, mut sx, mut sy) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (p, &w) in points.iter().zip(weights) {
        sw += w;
        sx += w * f64::from(p.x);
        sy += w * f64::from(p.y);
    }
    if sw <= f64::EPSILON {
        return (points[0], 0.0);
    }
    let mx = sx / sw;
    let my = sy / sw;
    let (mut sxx, mut sxy) = (0.0_f64, 0.0_f64);
    for (p, &w) in points.iter().zip(weights) {
        let dx = f64::from(p.x) - mx;
        let dy = f64::from(p.y) - my;
        sxx += w * dx * dx;
        sxy += w * dx * dy;
    }
    let slope = if sxx > 1e-6 { sxy / sxx } else { 0.0 };
    let origin_y = my + slope * (f64::from(min_x) - mx);
    (
        Point {
            x: min_x,
            y: origin_y as f32,
        },
        slope.atan() as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// Creates a letter glyph at `(x, baseline_y)` with unit height `h`, returning one or more strokes.
    fn letter_h(x: f32, baseline_y: f32, h: f32) -> Vec<Vec<Point>> {
        let w = h * 0.6;
        vec![
            vec![p(x, baseline_y - h), p(x, baseline_y)],
            vec![p(x + w, baseline_y - h), p(x + w, baseline_y)],
            vec![p(x, baseline_y - h * 0.5), p(x + w, baseline_y - h * 0.5)],
        ]
    }

    fn letter_i(x: f32, baseline_y: f32, h: f32) -> Vec<Vec<Point>> {
        vec![
            vec![p(x, baseline_y - h * 0.65), p(x, baseline_y)],
            // Small dot above the stem.
            vec![p(x, baseline_y - h * 0.9), p(x + 0.8, baseline_y - h * 0.9)],
        ]
    }

    fn letter_o(x: f32, baseline_y: f32, h: f32) -> Vec<Vec<Point>> {
        let r = h * 0.32;
        let cx = x + r;
        let cy = baseline_y - r;
        let pts = (0..=12)
            .map(|k| {
                let t = std::f32::consts::TAU * k as f32 / 12.0;
                p(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect();
        vec![pts]
    }

    fn letter_g_descender(x: f32, baseline_y: f32, h: f32) -> Vec<Vec<Point>> {
        let r = h * 0.3;
        vec![vec![
            p(x + r * 2.0, baseline_y - r * 2.0),
            p(x, baseline_y - r * 2.0),
            p(x, baseline_y),
            p(x + r * 2.0, baseline_y),
            p(x + r * 2.0, baseline_y + h * 0.45),
            p(x, baseline_y + h * 0.45),
        ]]
    }

    fn build_level_paragraph() -> Vec<Vec<Point>> {
        let h = 14.0;
        let mut strokes = Vec::new();
        for line in 0..3 {
            let base_y = 100.0 + line as f32 * 40.0;
            let mut x = 40.0;
            for word_idx in 0..5 {
                strokes.extend(letter_h(x, base_y, h));
                x += 14.0;
                strokes.extend(letter_i(x, base_y, h));
                x += 7.0;
                strokes.extend(letter_o(x, base_y, h));
                x += 14.0;
                if word_idx == 2 {
                    strokes.extend(letter_g_descender(x, base_y, h));
                    x += 14.0;
                }
                x += 8.0;
            }
        }
        strokes
    }

    fn rotate_all(strokes: &[Vec<Point>], pivot: Point, angle_deg: f32) -> Vec<Vec<Point>> {
        let rad = angle_deg.to_radians();
        strokes
            .iter()
            .map(|s| {
                s.iter()
                    .map(|&pt| rigid_transform(pt, pivot, rad, ZERO_POINT))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn sloped_paragraph_becomes_level_without_changing_letter_shapes() {
        let original = build_level_paragraph();
        let pivot = p(40.0, 100.0);
        let sloped = rotate_all(&original, pivot, 14.0);

        let result = straighten(&sloped, StraightenOptions::new());
        assert_eq!(result.strokes.len(), sloped.len());
        assert_eq!(
            result.baselines.len(),
            3,
            "should detect 3 lines in paragraph: {:?}",
            result.baselines
        );

        // Each detected baseline should closely match the 14° slope.
        for baseline in &result.baselines {
            let deg = baseline.angle_radians.to_degrees();
            assert!(
                (deg - 14.0).abs() < 1.5,
                "expected ~14° baseline slope, got {deg:.2}°"
            );
        }

        // Re-running straightening on the output should find ~0° slope on every line.
        let second = straighten(&result.strokes, StraightenOptions::new());
        assert_eq!(second.baselines.len(), 3);
        for baseline in &second.baselines {
            let residual_deg = baseline.angle_radians.to_degrees().abs();
            assert!(
                residual_deg < 0.6,
                "straightened line should be level, residual slope {residual_deg:.3}°"
            );
        }

        // Exact shape preservation: every pairwise segment length within every stroke is identical.
        for (before, after) in sloped.iter().zip(&result.strokes) {
            assert_eq!(before.len(), after.len());
            for (seg_b, seg_a) in before.windows(2).zip(after.windows(2)) {
                let len_b = (seg_b[1].x - seg_b[0].x).hypot(seg_b[1].y - seg_b[0].y);
                let len_a = (seg_a[1].x - seg_a[0].x).hypot(seg_a[1].y - seg_a[0].y);
                assert!(
                    (len_b - len_a).abs() < 1e-4,
                    "segment length changed: {len_b} vs {len_a}"
                );
            }
        }

        // Diacritic preservation: every `i` dot keeps its exact distance to its stem.
        // In each 5-word line, the first `i` starts at stroke index 3 (stem) and 4 (dot).
        let stem_before = sloped[3][0];
        let dot_before = sloped[4][0];
        let dist_before = (dot_before.x - stem_before.x).hypot(dot_before.y - stem_before.y);
        let stem_after = result.strokes[3][0];
        let dot_after = result.strokes[4][0];
        let dist_after = (dot_after.x - stem_after.x).hypot(dot_after.y - stem_after.y);
        assert!(
            (dist_before - dist_after).abs() < 1e-4,
            "i-dot shifted relative to stem: {dist_before} vs {dist_after}"
        );
    }

    #[test]
    fn negative_slope_and_even_spacing_level_cleanly() {
        let original = build_level_paragraph();
        let sloped = rotate_all(&original, p(40.0, 100.0), -16.0);
        let result = straighten(
            &sloped,
            StraightenOptions {
                max_angle_degrees: 45.0,
                spacing: LineSpacing::EvenSpacing,
            },
        );
        assert_eq!(result.baselines.len(), 3);
        for b in &result.baselines {
            assert!((b.angle_radians.to_degrees() + 16.0).abs() < 1.5);
        }
        let check = straighten(&result.strokes, StraightenOptions::new());
        for b in &check.baselines {
            assert!(b.angle_radians.to_degrees().abs() < 0.6);
        }
    }

    #[test]
    fn ink_points_keep_their_per_point_widths_and_empty_input_is_safe() {
        let empty = straighten::<Vec<Point>>(&[], StraightenOptions::new());
        assert!(empty.strokes.is_empty());
        assert!(empty.baselines.is_empty());

        let stroke_a = vec![
            InkPoint {
                x: 10.0,
                y: 20.0,
                width: 1.25,
            },
            InkPoint {
                x: 10.0,
                y: 34.0,
                width: 2.75,
            },
        ];
        let stroke_b = vec![
            InkPoint {
                x: 60.0,
                y: 30.0,
                width: 1.5,
            },
            InkPoint {
                x: 60.0,
                y: 44.0,
                width: 3.1,
            },
        ];
        let (out, baselines) = straighten_ink(&[stroke_a, stroke_b], StraightenOptions::new());
        assert_eq!(baselines.len(), 1);
        assert_eq!(out[0][0].width, 1.25);
        assert_eq!(out[0][1].width, 2.75);
        assert_eq!(out[1][0].width, 1.5);
        assert_eq!(out[1][1].width, 3.1);
        // Bottoms of both strokes should now sit at the same y coordinate.
        assert!((out[0][1].y - out[1][1].y).abs() < 0.1);
    }
}
