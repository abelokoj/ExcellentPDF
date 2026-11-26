//! Projective 3×3 homography solver, 4-corner perspective warp, and keystone crop (plan O.21).

use super::types::{
    CameraScanError, KeystoneTarget, Point2D, QuadCorners, RawImage, Result, ScanPixelFormat,
};

/// 3×3 row-major projective homography matrix (`H`) mapping destination `(u, v, 1)` to source `(x, y, w)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    m: [f64; 9],
}

impl Homography {
    /// Applies the projective transform to destination point `(u, v)`.
    #[must_use]
    pub fn transform_point(&self, u: f64, v: f64) -> Option<Point2D> {
        let w = self.m[6] * u + self.m[7] * v + self.m[8];
        if !w.is_finite() || w.abs() < 1e-9 {
            return None;
        }
        Some(Point2D::new(
            (self.m[0] * u + self.m[1] * v + self.m[2]) / w,
            (self.m[3] * u + self.m[4] * v + self.m[5]) / w,
        ))
    }

    /// Returns the raw 3×3 row-major matrix entries.
    #[must_use]
    pub const fn matrix(&self) -> [f64; 9] {
        self.m
    }
}

/// Computes the inverse projective homography mapping destination rectangle `(0,0)..(dst_width, dst_height)`
/// onto source quadrilateral `src` via an exact 8×8 Direct Linear Transform (DLT) solve.
///
/// # Errors
///
/// Returns [`CameraScanError::DegenerateQuad`] if `src` is non-convex, zero-area, or yields a singular system.
pub fn compute_homography(src: QuadCorners, dst_width: u32, dst_height: u32) -> Result<Homography> {
    if dst_width == 0 || dst_height == 0 {
        return Err(CameraScanError::InvalidImage(
            "target warp dimensions must be positive".to_owned(),
        ));
    }
    if !src.is_convex() || src.area() < 1.0 {
        return Err(CameraScanError::DegenerateQuad(
            "source quad is non-convex or has negligible area".to_owned(),
        ));
    }

    let w = f64::from(dst_width.saturating_sub(1).max(1));
    let h = f64::from(dst_height.saturating_sub(1).max(1));
    let dst = [
        Point2D::new(0.0, 0.0),
        Point2D::new(w, 0.0),
        Point2D::new(w, h),
        Point2D::new(0.0, h),
    ];
    let src_pts = src.as_array();

    // Solve 8x8 linear system A * x = b for [h00, h01, h02, h10, h11, h12, h20, h21], with h22 = 1.
    let mut a = [[0.0_f64; 9]; 8];
    for i in 0..4 {
        let (u, v) = (dst[i].x, dst[i].y);
        let (x, y) = (src_pts[i].x, src_pts[i].y);
        let r0 = 2 * i;
        let r1 = r0 + 1;
        a[r0] = [u, v, 1.0, 0.0, 0.0, 0.0, -u * x, -v * x, x];
        a[r1] = [0.0, 0.0, 0.0, u, v, 1.0, -u * y, -v * y, y];
    }

    let coeffs = solve_linear_8x8(a)?;
    Ok(Homography {
        m: [
            coeffs[0], coeffs[1], coeffs[2], coeffs[3], coeffs[4], coeffs[5], coeffs[6], coeffs[7],
            1.0,
        ],
    })
}

/// Applies 4-corner perspective warp and keystone crop to `image`, returning a rectified `Rgb8` image.
///
/// # Errors
///
/// Returns [`CameraScanError`] if `quad` is degenerate or output dimensions are invalid.
pub fn perspective_warp(
    image: &RawImage,
    quad: QuadCorners,
    target: KeystoneTarget,
) -> Result<RawImage> {
    let (nat_w, nat_h) = quad.natural_dimensions();
    let (dst_w, dst_h) = match target {
        KeystoneTarget::AutoAspect => (nat_w, nat_h),
        KeystoneTarget::A4Portrait => {
            let h = nat_h.max(nat_w);
            let w = ((f64::from(h) / std::f64::consts::SQRT_2).round() as u32).max(1);
            (w, h)
        }
        KeystoneTarget::LetterPortrait => {
            let h = nat_h.max(nat_w);
            let w = ((f64::from(h) * (8.5 / 11.0)).round() as u32).max(1);
            (w, h)
        }
        KeystoneTarget::ExplicitPixels { width, height } => (width, height),
    };

    let homography = compute_homography(quad, dst_w, dst_h)?;
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 3];
    let src_max_x = f64::from(image.width() - 1);
    let src_max_y = f64::from(image.height() - 1);

    for v in 0..dst_h {
        for u in 0..dst_w {
            let Some(src_pt) = homography.transform_point(f64::from(u), f64::from(v)) else {
                continue;
            };
            let sx = src_pt.x.clamp(0.0, src_max_x);
            let sy = src_pt.y.clamp(0.0, src_max_y);
            let rgb = sample_bilinear(image, sx, sy);
            let idx = ((v as usize) * (dst_w as usize) + (u as usize)) * 3;
            out[idx..idx + 3].copy_from_slice(&rgb);
        }
    }

    RawImage::try_new(dst_w, dst_h, ScanPixelFormat::Rgb8, out)
}

fn sample_bilinear(image: &RawImage, x: f64, y: f64) -> [u8; 3] {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let fx = x - f64::from(x0);
    let fy = y - f64::from(y0);

    let p00 = image.pixel_rgb(x0, y0);
    let p10 = image.pixel_rgb(x1, y0);
    let p01 = image.pixel_rgb(x0, y1);
    let p11 = image.pixel_rgb(x1, y1);

    let mut out = [0u8; 3];
    for c in 0..3 {
        let top = f64::from(p00[c]) * (1.0 - fx) + f64::from(p10[c]) * fx;
        let bot = f64::from(p01[c]) * (1.0 - fx) + f64::from(p11[c]) * fx;
        let val = top * (1.0 - fy) + bot * fy;
        out[c] = val.round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn solve_linear_8x8(mut a: [[f64; 9]; 8]) -> Result<[f64; 8]> {
    for col in 0..8 {
        let mut pivot_row = col;
        let mut max_val = a[col][col].abs();
        for (row, r_vals) in a.iter().enumerate().skip(col + 1) {
            let v = r_vals[col].abs();
            if v > max_val {
                max_val = v;
                pivot_row = row;
            }
        }
        if max_val < 1e-10 {
            return Err(CameraScanError::DegenerateQuad(
                "homography linear system is singular".to_owned(),
            ));
        }
        if pivot_row != col {
            a.swap(col, pivot_row);
        }
        let diag = a[col][col];
        for item in &mut a[col][col..9] {
            *item /= diag;
        }
        let pivot_snapshot = a[col];
        for (row, target_row) in a.iter_mut().enumerate() {
            if row == col {
                continue;
            }
            let factor = target_row[col];
            if factor.abs() < 1e-15 {
                continue;
            }
            for c in col..9 {
                target_row[c] -= factor * pivot_snapshot[c];
            }
        }
    }

    let mut sol = [0.0_f64; 8];
    for i in 0..8 {
        sol[i] = a[i][8];
    }
    Ok(sol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_maps_corners_and_rectifies_skewed_quad() {
        let w = 120u32;
        let h = 100u32;
        let mut rgb = vec![20u8; (w * h * 3) as usize];
        let quad = QuadCorners {
            top_left: Point2D::new(20.0, 15.0),
            top_right: Point2D::new(100.0, 22.0),
            bottom_right: Point2D::new(108.0, 88.0),
            bottom_left: Point2D::new(12.0, 82.0),
        };
        // Fill interior of quad with bright paper color (230, 230, 225).
        for y in 0..h {
            for x in 0..w {
                let pt = Point2D::new(x as f64, y as f64);
                let arr = quad.as_array();
                let inside = (0..4).all(|i| {
                    let a = arr[i];
                    let b = arr[(i + 1) % 4];
                    (b.x - a.x) * (pt.y - a.y) - (b.y - a.y) * (pt.x - a.x) >= 0.0
                });
                if inside {
                    let idx = ((y * w + x) * 3) as usize;
                    rgb[idx] = 230;
                    rgb[idx + 1] = 230;
                    rgb[idx + 2] = 225;
                }
            }
        }
        let img = RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb).expect("img");
        let warped = perspective_warp(
            &img,
            quad,
            KeystoneTarget::ExplicitPixels {
                width: 80,
                height: 60,
            },
        )
        .expect("warp");
        assert_eq!((warped.width(), warped.height()), (80, 60));
        // Center pixel must sample the bright document interior, not dark desk background.
        let center = warped.pixel_rgb(40, 30);
        assert!(center[0] > 200 && center[1] > 200, "center={center:?}");
    }
}
