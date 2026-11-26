//! Sobel gradient edge detection, convex hull simplification, and quad contour candidate ranking
//! for document boundary localization (plan O.21).

use super::types::{CameraScanError, Point2D, QuadCorners, RawImage, Result};

/// Sobel gradient magnitude map computed over a Gaussian-smoothed luminance image.
#[derive(Debug, Clone, PartialEq)]
pub struct SobelEdgeMap {
    width: u32,
    height: u32,
    magnitudes: Vec<u16>,
    threshold: u16,
}

impl SobelEdgeMap {
    /// Computes Sobel edge magnitudes on `image` after 3×3 Gaussian smoothing.
    ///
    /// # Errors
    ///
    /// Returns [`CameraScanError::InvalidImage`] if the image is smaller than `3 × 3`.
    pub fn compute(image: &RawImage) -> Result<Self> {
        let w = image.width();
        let h = image.height();
        if w < 3 || h < 3 {
            return Err(CameraScanError::InvalidImage(
                "edge detection requires at least a 3x3 image".to_owned(),
            ));
        }

        let luma = image.to_luma();
        let blurred = box_gaussian_3x3(&luma, w as usize, h as usize);
        let w_usize = w as usize;
        let h_usize = h as usize;
        let mut magnitudes = vec![0u16; w_usize * h_usize];
        let mut histogram = [0usize; 1025];

        for y in 1..h_usize - 1 {
            for x in 1..w_usize - 1 {
                let idx = |dx: isize, dy: isize| -> i32 {
                    let xx = (x as isize + dx) as usize;
                    let yy = (y as isize + dy) as usize;
                    i32::from(blurred[yy * w_usize + xx])
                };
                let gx = -idx(-1, -1) + idx(1, -1) - 2 * idx(-1, 0) + 2 * idx(1, 0) - idx(-1, 1)
                    + idx(1, 1);
                let gy = -idx(-1, -1) - 2 * idx(0, -1) - idx(1, -1)
                    + idx(-1, 1)
                    + 2 * idx(0, 1)
                    + idx(1, 1);
                let mag = ((gx.abs() + gy.abs()) as u32).min(1024) as u16;
                magnitudes[y * w_usize + x] = mag;
                histogram[mag as usize] += 1;
            }
        }

        let threshold = otsu_u16(&histogram, (w_usize - 2) * (h_usize - 2)).max(24);
        Ok(Self {
            width: w,
            height: h,
            magnitudes,
            threshold,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Adaptive edge threshold.
    #[must_use]
    pub const fn threshold(&self) -> u16 {
        self.threshold
    }

    /// Gradient magnitude at `(x, y)`.
    #[must_use]
    pub fn magnitude(&self, x: u32, y: u32) -> u16 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.magnitudes[y as usize * self.width as usize + x as usize]
    }
}

/// Ranked document quadrilateral candidate detected from Sobel edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadCandidate {
    /// Four corners in canonical order.
    pub corners: QuadCorners,
    /// Overall detection confidence (`0.0..=1.0`).
    pub confidence: f64,
    /// Fraction of sampled perimeter points supported by strong Sobel gradients.
    pub edge_support: f64,
    /// Fraction of the image area enclosed by the quad.
    pub area_ratio: f64,
}

/// Detects ranked 4-corner document quadrilateral candidates in `image` (plan O.21).
///
/// Uses Sobel gradient extraction, adaptive Otsu thresholding, monotone-chain convex hull,
/// Visvalingam-style area-preserving polygon reduction to 4 corners, and perimeter edge-support scoring.
///
/// # Errors
///
/// Returns [`CameraScanError::InvalidImage`] if `image` is too small for edge detection.
pub fn detect_document_quads(image: &RawImage) -> Result<Vec<QuadCandidate>> {
    let edges = SobelEdgeMap::compute(image)?;
    let w = image.width() as usize;
    let h = image.height() as usize;
    let total_area = (w * h) as f64;

    let mut edge_points = Vec::new();
    let thresh = edges.threshold();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if edges.magnitudes[y * w + x] >= thresh {
                edge_points.push(Point2D::new(x as f64, y as f64));
            }
        }
    }

    if edge_points.len() < 8 {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    let hull = convex_hull(&edge_points);
    if hull.len() >= 4 {
        let reduced = reduce_polygon_to_quad(&hull);
        if let Some(corners) = validate_and_score(&reduced, &edges, total_area) {
            candidates.push(corners);
        }
    }

    // Secondary candidate from extremal diagonals (`x+y`, `x-y`) on strong edge pixels.
    let extremal = extremal_quad(&edge_points);
    if let Some(ext_cand) = validate_and_score(&extremal, &edges, total_area) {
        let duplicate = candidates.iter().any(|c| {
            c.corners.top_left.distance(ext_cand.corners.top_left) < 4.0
                && c.corners
                    .bottom_right
                    .distance(ext_cand.corners.bottom_right)
                    < 4.0
        });
        if !duplicate {
            candidates.push(ext_cand);
        }
    }

    candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    Ok(candidates)
}

fn validate_and_score(
    corners: &QuadCorners,
    edges: &SobelEdgeMap,
    total_area: f64,
) -> Option<QuadCandidate> {
    if !corners.is_convex() {
        return None;
    }
    let area = corners.area();
    let area_ratio = area / total_area.max(1.0);
    if !(0.05..=0.99).contains(&area_ratio) {
        return None;
    }
    let edge_support = measure_perimeter_support(corners, edges);
    let area_score = if area_ratio > 0.2 {
        1.0
    } else {
        area_ratio / 0.2
    };
    let confidence = (0.75 * edge_support + 0.25 * area_score).clamp(0.0, 1.0);
    Some(QuadCandidate {
        corners: *corners,
        confidence,
        edge_support,
        area_ratio,
    })
}

fn measure_perimeter_support(quad: &QuadCorners, edges: &SobelEdgeMap) -> f64 {
    let pts = quad.as_array();
    let mut hits = 0usize;
    let mut samples = 0usize;
    let thresh = edges.threshold() / 2;

    for i in 0..4 {
        let a = pts[i];
        let b = pts[(i + 1) % 4];
        let steps = (a.distance(b).ceil() as usize).clamp(8, 256);
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let px = (a.x + (b.x - a.x) * t).round() as i32;
            let py = (a.y + (b.y - a.y) * t).round() as i32;
            samples += 1;
            let mut max_local = 0u16;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let sx = (px + dx).clamp(0, edges.width() as i32 - 1) as u32;
                    let sy = (py + dy).clamp(0, edges.height() as i32 - 1) as u32;
                    max_local = max_local.max(edges.magnitude(sx, sy));
                }
            }
            if max_local >= thresh {
                hits += 1;
            }
        }
    }

    if samples == 0 {
        0.0
    } else {
        hits as f64 / samples as f64
    }
}

fn extremal_quad(points: &[Point2D]) -> QuadCorners {
    let mut tl = points[0];
    let mut br = points[0];
    let mut tr = points[0];
    let mut bl = points[0];

    for &p in points {
        if p.x + p.y < tl.x + tl.y {
            tl = p;
        }
        if p.x + p.y > br.x + br.y {
            br = p;
        }
        if p.x - p.y > tr.x - tr.y {
            tr = p;
        }
        if p.x - p.y < bl.x - bl.y {
            bl = p;
        }
    }
    QuadCorners::from_unordered([tl, tr, br, bl])
}

fn box_gaussian_3x3(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = src.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let acc = u32::from(src[(y - 1) * w + (x - 1)])
                + 2 * u32::from(src[(y - 1) * w + x])
                + u32::from(src[(y - 1) * w + (x + 1)])
                + 2 * u32::from(src[y * w + (x - 1)])
                + 4 * u32::from(src[y * w + x])
                + 2 * u32::from(src[y * w + (x + 1)])
                + u32::from(src[(y + 1) * w + (x - 1)])
                + 2 * u32::from(src[(y + 1) * w + x])
                + u32::from(src[(y + 1) * w + (x + 1)]);
            out[y * w + x] = (acc >> 4) as u8;
        }
    }
    out
}

fn otsu_u16(hist: &[usize; 1025], total: usize) -> u16 {
    if total == 0 {
        return 32;
    }
    let mut sum_all = 0.0_f64;
    for (i, &count) in hist.iter().enumerate() {
        sum_all += (i as f64) * (count as f64);
    }
    let mut sum_b = 0.0_f64;
    let mut w_b = 0usize;
    let mut best_var = -1.0_f64;
    let mut best_t = 32u16;

    for (t, &count) in hist.iter().enumerate() {
        w_b += count;
        if w_b == 0 {
            continue;
        }
        let w_f = total.saturating_sub(w_b);
        if w_f == 0 {
            break;
        }
        sum_b += (t as f64) * (count as f64);
        let m_b = sum_b / (w_b as f64);
        let m_f = (sum_all - sum_b) / (w_f as f64);
        let between = (w_b as f64) * (w_f as f64) * (m_b - m_f) * (m_b - m_f);
        if between > best_var {
            best_var = between;
            best_t = t as u16;
        }
    }
    best_t
}

/// Andrew's monotone-chain 2D convex hull.
fn convex_hull(points: &[Point2D]) -> Vec<Point2D> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5);
    if pts.len() < 3 {
        return pts;
    }

    let cross = |o: Point2D, a: Point2D, b: Point2D| -> f64 {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    };

    let mut lower = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }

    let mut upper = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }

    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Iteratively removes the hull vertex contributing the smallest triangle area until 4 corners remain.
fn reduce_polygon_to_quad(hull: &[Point2D]) -> QuadCorners {
    let mut poly = hull.to_vec();
    while poly.len() > 4 {
        let n = poly.len();
        let mut min_area = f64::INFINITY;
        let mut min_idx = 0usize;
        for i in 0..n {
            let a = poly[(i + n - 1) % n];
            let b = poly[i];
            let c = poly[(i + 1) % n];
            let tri_area = ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() * 0.5;
            if tri_area < min_area {
                min_area = tri_area;
                min_idx = i;
            }
        }
        poly.remove(min_idx);
    }
    QuadCorners::from_unordered([poly[0], poly[1], poly[2], poly[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_scan::types::ScanPixelFormat;

    #[test]
    fn detects_skewed_bright_page_quad_on_dark_desk_background() {
        let w = 160u32;
        let h = 120u32;
        let mut rgb = vec![25u8; (w * h * 3) as usize];

        let expected = QuadCorners {
            top_left: Point2D::new(30.0, 20.0),
            top_right: Point2D::new(130.0, 28.0),
            bottom_right: Point2D::new(140.0, 102.0),
            bottom_left: Point2D::new(22.0, 96.0),
        };
        let poly = expected.as_array();

        for y in 0..h {
            for x in 0..w {
                if point_in_convex_quad(Point2D::new(x as f64, y as f64), &poly) {
                    let idx = ((y * w + x) * 3) as usize;
                    rgb[idx] = 240;
                    rgb[idx + 1] = 240;
                    rgb[idx + 2] = 235;
                }
            }
        }

        let img = RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb).expect("valid image");
        let quads = detect_document_quads(&img).expect("quad detection");
        assert!(!quads.is_empty(), "expected at least one quad candidate");
        let best = quads[0];
        assert!(best.confidence > 0.6, "confidence={}", best.confidence);
        assert!(
            best.corners.top_left.distance(expected.top_left) < 4.0,
            "tl={:?}",
            best.corners.top_left
        );
        assert!(
            best.corners.bottom_right.distance(expected.bottom_right) < 4.0,
            "br={:?}",
            best.corners.bottom_right
        );
    }

    fn point_in_convex_quad(p: Point2D, poly: &[Point2D; 4]) -> bool {
        let mut pos = false;
        let mut neg = false;
        for i in 0..4 {
            let a = poly[i];
            let b = poly[(i + 1) % 4];
            let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            if cross > 0.0 {
                pos = true;
            } else if cross < 0.0 {
                neg = true;
            }
        }
        !(pos && neg)
    }
}
