//! Adaptive whiteboard and document contrast cleanup using integral-image local illumination
//! estimation and Sauvola adaptive binarization (plan O.21).

use super::types::{RawImage, Result, ScanPixelFormat};

/// Contrast & background cleanup preset applied to rectified scanned pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CleanupPreset {
    /// Keep rectified pixels unchanged (normalized to `Rgb8`).
    Original,
    /// Normalize uneven shadows/vignette while preserving color graphics and highlights.
    #[default]
    DocumentColor,
    /// Aggressive whiteboard illumination flattening with marker chroma boost on bright white.
    Whiteboard,
    /// Shadow-normalized clean grayscale document (`Rgb8` with `R == G == B`).
    Grayscale,
    /// Crisp Sauvola adaptive black-and-white text binarization.
    BlackAndWhite,
}

/// Applies `preset` to `image`, returning a cleaned `Rgb8` [`RawImage`].
///
/// # Errors
///
/// Returns [`super::types::CameraScanError::InvalidImage`] if output buffer construction fails.
pub fn enhance_document(image: &RawImage, preset: CleanupPreset) -> Result<RawImage> {
    let w = image.width();
    let h = image.height();
    let rgb = image.to_rgb8_bytes();

    match preset {
        CleanupPreset::Original => RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb),
        CleanupPreset::DocumentColor => enhance_illumination(&rgb, w, h, 1.15, 0.92),
        CleanupPreset::Whiteboard => enhance_whiteboard(&rgb, w, h),
        CleanupPreset::Grayscale => enhance_grayscale(image),
        CleanupPreset::BlackAndWhite => enhance_sauvola(&image.to_luma(), w, h),
    }
}

struct IntegralLuma {
    width: usize,
    height: usize,
    sum: Vec<u64>,
    sq_sum: Vec<u64>,
}

impl IntegralLuma {
    fn from_luma(luma: &[u8], width: usize, height: usize) -> Self {
        let stride = width + 1;
        let mut sum = vec![0u64; stride * (height + 1)];
        let mut sq_sum = vec![0u64; stride * (height + 1)];
        for y in 0..height {
            let mut row_sum = 0u64;
            let mut row_sq = 0u64;
            for x in 0..width {
                let v = u64::from(luma[y * width + x]);
                row_sum += v;
                row_sq += v * v;
                let idx = (y + 1) * stride + (x + 1);
                sum[idx] = sum[y * stride + (x + 1)] + row_sum;
                sq_sum[idx] = sq_sum[y * stride + (x + 1)] + row_sq;
            }
        }
        Self {
            width,
            height,
            sum,
            sq_sum,
        }
    }

    fn window_stats(&self, cx: usize, cy: usize, radius: usize) -> (f64, f64) {
        let x0 = cx.saturating_sub(radius);
        let y0 = cy.saturating_sub(radius);
        let x1 = (cx + radius + 1).min(self.width);
        let y1 = (cy + radius + 1).min(self.height);
        let area = ((x1 - x0) * (y1 - y0)).max(1) as f64;
        let stride = self.width + 1;

        let rect_sum = |table: &[u64]| -> f64 {
            let total = table[y1 * stride + x1] + table[y0 * stride + x0]
                - table[y0 * stride + x1]
                - table[y1 * stride + x0];
            total as f64
        };
        let mean = rect_sum(&self.sum) / area;
        let sq_mean = rect_sum(&self.sq_sum) / area;
        let var = (sq_mean - mean * mean).max(0.0);
        (mean, var.sqrt())
    }
}

fn enhance_illumination(
    rgb: &[u8],
    w: u32,
    h: u32,
    gain: f64,
    white_floor: f64,
) -> Result<RawImage> {
    let w_usize = w as usize;
    let h_usize = h as usize;
    let luma: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|px| {
            ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8) as u8
        })
        .collect();
    let sat = IntegralLuma::from_luma(&luma, w_usize, h_usize);
    let radius = ((w_usize.min(h_usize)) / 10).clamp(6, 36);
    let mut out = Vec::with_capacity(rgb.len());

    for y in 0..h_usize {
        for x in 0..w_usize {
            let (local_mean, local_std) = sat.window_stats(x, y, radius);
            // Estimate local paper background slightly above local mean when text is present.
            let bg = (local_mean + 0.5 * local_std).clamp(48.0, 255.0);
            let idx = (y * w_usize + x) * 3;
            for c in 0..3 {
                let raw = f64::from(rgb[idx + c]);
                let norm = (raw / bg) * 255.0 * gain;
                let snapped = if norm > 255.0 * white_floor {
                    255.0
                } else {
                    norm
                };
                out.push(snapped.round().clamp(0.0, 255.0) as u8);
            }
        }
    }

    RawImage::try_new(w, h, ScanPixelFormat::Rgb8, out)
}

fn enhance_whiteboard(rgb: &[u8], w: u32, h: u32) -> Result<RawImage> {
    let flat = enhance_illumination(rgb, w, h, 1.25, 0.82)?;
    let mut out = Vec::with_capacity(flat.data().len());
    for px in flat.data().as_chunks::<3>().0 {
        let min_c = px[0].min(px[1]).min(px[2]);
        let max_c = px[0].max(px[1]).max(px[2]);
        if min_c > 195 && (max_c - min_c) < 28 {
            out.extend_from_slice(&[255, 255, 255]);
        } else {
            // Boost marker ink contrast against white board.
            let mean = (u16::from(px[0]) + u16::from(px[1]) + u16::from(px[2])) as f32 / 3.0;
            for &ch in px {
                let boosted = mean + (f32::from(ch) - mean) * 1.45;
                out.push((boosted * 0.92).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    RawImage::try_new(w, h, ScanPixelFormat::Rgb8, out)
}

fn enhance_grayscale(image: &RawImage) -> Result<RawImage> {
    let w = image.width();
    let h = image.height();
    let cleaned = enhance_illumination(&image.to_rgb8_bytes(), w, h, 1.15, 0.90)?;
    let mut out = Vec::with_capacity(cleaned.data().len());
    for px in cleaned.data().as_chunks::<3>().0 {
        let y =
            ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8) as u8;
        out.extend_from_slice(&[y, y, y]);
    }
    RawImage::try_new(w, h, ScanPixelFormat::Rgb8, out)
}

fn enhance_sauvola(luma: &[u8], w: u32, h: u32) -> Result<RawImage> {
    let w_usize = w as usize;
    let h_usize = h as usize;
    let sat = IntegralLuma::from_luma(luma, w_usize, h_usize);
    let radius = ((w_usize.min(h_usize)) / 14).clamp(4, 24);
    let k = 0.20_f64;
    let mut out = Vec::with_capacity(w_usize * h_usize * 3);

    for y in 0..h_usize {
        for x in 0..w_usize {
            let (mean, std_dev) = sat.window_stats(x, y, radius);
            let threshold = mean * (1.0 + k * (std_dev / 128.0 - 1.0));
            let px = if f64::from(luma[y * w_usize + x]) >= threshold {
                255
            } else {
                0
            };
            out.extend_from_slice(&[px, px, px]);
        }
    }

    RawImage::try_new(w, h, ScanPixelFormat::Rgb8, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whiteboard_cleanup_lifts_shaded_background_and_preserves_dark_strokes() {
        let w = 64u32;
        let h = 64u32;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                // Uneven lighting gradient from 130..200 across the board, with dark stroke in center.
                let bg = (130 + x) as u8;
                if (28..36).contains(&x) && (16..48).contains(&y) {
                    rgb.extend_from_slice(&[20, 40, 140]);
                } else {
                    rgb.extend_from_slice(&[bg, bg, bg]);
                }
            }
        }
        let img = RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb).expect("img");
        let cleaned = enhance_document(&img, CleanupPreset::Whiteboard).expect("whiteboard");
        let corner = cleaned.pixel_rgb(2, 2);
        let stroke = cleaned.pixel_rgb(32, 32);
        assert!(corner[0] >= 240, "shaded board should whiten: {corner:?}");
        assert!(
            stroke[0] < 80,
            "marker stroke should remain dark: {stroke:?}"
        );

        let bw = enhance_document(&img, CleanupPreset::BlackAndWhite).expect("bw");
        assert_eq!(bw.pixel_rgb(2, 2), [255, 255, 255]);
        assert_eq!(bw.pixel_rgb(32, 32), [0, 0, 0]);
    }
}
