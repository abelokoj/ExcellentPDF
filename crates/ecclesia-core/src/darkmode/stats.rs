//! What a small sample of an image says about it: photo, artwork, or a light panel that belongs to the page
//! (Enhanced's `PdfDarkModeImageStats.cpp`, its sampling and every threshold kept).

/// Samples taken along each edge for the border statistics.
const BORDER_SAMPLES_PER_EDGE: usize = 32;
/// A border this light all round, and this flat, is a backdrop (a chart's or a 3D render's), which follows the page.
const LIGHT_BACKDROP_BORDER_LIGHT: f32 = 0.95;
const LIGHT_BACKDROP_BORDER_UNIFORMITY: f32 = 0.90;
/// Luminance above which a pixel counts as light.
const LIGHT: f32 = 0.72;

/// Colour statistics of an image sample.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SampleStats {
    /// 4-bit-per-channel colour buckets holding more than 1 % of the samples.
    pub significant_buckets: u32,
    /// Variance of the luminance.
    pub lum_var: f32,
    /// Share of samples whose channels differ by more than 0.12.
    pub sat_ratio: f32,
    /// Share of light samples.
    pub high_lum_ratio: f32,
    /// Share of light samples along the border.
    pub border_light_ratio: f32,
    /// 1 for a border of one flat colour, falling to 0 as it varies.
    pub border_uniformity: f32,
}

impl SampleStats {
    /// Measures an RGB sample (`components` bytes a pixel, red first; rows `stride` bytes apart). `None` when it
    /// has no pixels.
    pub fn measure(
        samples: &[u8],
        width: usize,
        height: usize,
        stride: usize,
        components: usize,
    ) -> Option<Self> {
        if width == 0 || height == 0 || components < 3 {
            return None;
        }
        let rgb = |x: usize, y: usize| {
            let at = y * stride + x * components;
            [0, 1, 2].map(|i| f32::from(samples[at + i]) / 255.0)
        };
        let mut buckets = vec![0u32; 4096];
        let (mut n, mut saturated, mut high) = (0u32, 0u32, 0u32);
        let (mut lum_sum, mut lum_sq_sum) = (0.0f32, 0.0f32);
        for y in (0..height).step_by(step(height)) {
            for x in (0..width).step_by(step(width)) {
                let at = y * stride + x * components;
                let [r, g, b] = [0, 1, 2].map(|i| usize::from(samples[at + i] >> 4));
                buckets[(r << 8) | (g << 4) | b] += 1;
                let [r, g, b] = rgb(x, y);
                let lum = luminance(r, g, b);
                lum_sum += lum;
                lum_sq_sum += lum * lum;
                if r.max(g).max(b) - r.min(g).min(b) > 0.12 {
                    saturated += 1;
                }
                if lum > LIGHT {
                    high += 1;
                }
                n += 1;
            }
        }
        let count = n as f32;
        let mean = lum_sum / count;
        let (border_light_ratio, border_uniformity) = border(width, height, &rgb);
        Some(Self {
            significant_buckets: buckets
                .iter()
                .filter(|&&b| b * 100 > n)
                .count()
                .try_into()
                .expect("at most 4096 buckets"),
            lum_var: lum_sq_sum / count - mean * mean,
            sat_ratio: saturated as f32 / count,
            high_lum_ratio: high as f32 / count,
            border_light_ratio,
            border_uniformity,
        })
    }

    /// Many colours, strong saturation or varied light: a photo, unless it is mostly light and dull.
    pub fn looks_like_photo(&self) -> bool {
        let mut photo =
            self.significant_buckets >= 16 || self.sat_ratio >= 0.18 || self.lum_var >= 0.014;
        if self.high_lum_ratio > 0.58 && self.sat_ratio < 0.18 {
            photo = false;
        }
        if self.significant_buckets <= 12 && self.lum_var < 0.012 && self.high_lum_ratio > 0.45 {
            photo = false;
        }
        if self.high_lum_ratio > 0.72 && self.sat_ratio < 0.18 {
            photo = false;
        }
        photo
    }

    /// A dark painting or picture, big enough on the page (`coverage`, its share of the page) to matter.
    pub fn looks_like_dark_artwork(&self, coverage: f32) -> bool {
        coverage >= 0.035
            && self.high_lum_ratio < 0.48
            && self.lum_var >= 0.004
            && (self.significant_buckets >= 8 || self.sat_ratio >= 0.08)
    }

    /// Whether a picture covering `coverage` of its page keeps its colours (Enhanced's `IsConfirmedArtwork`): layout
    /// panels and light backdrops follow the page, artwork and photos don't.
    pub fn keeps_colors(&self, coverage: f32) -> bool {
        if self.looks_like_flat_layout_panel()
            || self.looks_like_layout_background()
            || self.looks_like_light_backdrop()
        {
            return false;
        }
        if self.looks_like_dark_artwork(coverage) {
            return true;
        }
        self.looks_like_photo() && !(coverage < 0.14 && self.looks_like_paper_text_box())
    }

    fn looks_like_flat_layout_panel(&self) -> bool {
        self.high_lum_ratio > 0.76
            && self.lum_var < 0.011
            && self.significant_buckets <= 11
            && self.sat_ratio < 0.17
    }

    /// Cream, tan and yellow textbook panels and title cards.
    fn looks_like_layout_background(&self) -> bool {
        self.looks_like_flat_layout_panel()
            || (self.high_lum_ratio > 0.58 && self.lum_var < 0.018)
            || (self.high_lum_ratio > 0.44 && self.lum_var < 0.022)
            || (self.high_lum_ratio > 0.50
                && self.lum_var < 0.038
                && self.sat_ratio < 0.22
                && self.significant_buckets <= 14)
    }

    fn looks_like_light_backdrop(&self) -> bool {
        self.border_light_ratio >= LIGHT_BACKDROP_BORDER_LIGHT
            && self.border_uniformity >= LIGHT_BACKDROP_BORDER_UNIFORMITY
    }

    fn looks_like_paper_text_box(&self) -> bool {
        self.high_lum_ratio > 0.64
            && self.lum_var < 0.014
            && self.significant_buckets <= 12
            && self.sat_ratio < 0.20
    }
}

/// Whether a picture that fills most of its page (`coverage` ≥ 0.75) recolours with it: a scan or a full-bleed
/// background does; artwork, such as a cover, doesn't. A picture that couldn't be sampled (`None`) recolours.
pub(super) fn page_dominant_recolors(stats: Option<&SampleStats>, coverage: f32) -> bool {
    coverage >= 0.75 && !stats.is_some_and(|s| s.looks_like_dark_artwork(coverage))
}

/// Rec. 709 luminance of non-linear sRGB, as Enhanced uses it.
fn luminance(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// About 32 samples across `len` pixels.
fn step(len: usize) -> usize {
    if len >= 32 { len / 32 } else { 1 }
}

/// The border's share of light samples and its uniformity.
fn border(width: usize, height: usize, rgb: &impl Fn(usize, usize) -> [f32; 3]) -> (f32, f32) {
    let mut ring = Vec::with_capacity(BORDER_SAMPLES_PER_EDGE * 4);
    let mut take = |x, y| {
        if ring.len() < BORDER_SAMPLES_PER_EDGE * 4 {
            ring.push(rgb(x, y));
        }
    };
    for x in (0..width).step_by(step(width)) {
        take(x, 0);
        take(x, height - 1);
    }
    for y in (0..height).step_by(step(height)) {
        take(0, y);
        take(width - 1, y);
    }
    let n = ring.len() as f32;
    let light = ring
        .iter()
        .filter(|&&[r, g, b]| luminance(r, g, b) > LIGHT)
        .count() as f32;
    let mean = ring
        .iter()
        .fold([0.0f32; 3], |m, c| [m[0] + c[0], m[1] + c[1], m[2] + c[2]]);
    let mean = mean.map(|v| v / n);
    let var = ring
        .iter()
        .map(|c| (0..3).map(|i| (c[i] - mean[i]).powi(2)).sum::<f32>())
        .sum::<f32>()
        / n;
    (light / n, (1.0 - var / 0.12).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` × `h` RGB image whose pixel at `(x, y)` is `paint(x, y)`.
    fn image(w: usize, h: usize, paint: impl Fn(usize, usize) -> [u8; 3]) -> SampleStats {
        let mut samples = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                samples.extend(paint(x, y));
            }
        }
        SampleStats::measure(&samples, w, h, w * 3, 3).expect("pixels")
    }

    /// Patches of many colours, 8 pixels square, like a photo's areas of colour (a smooth gradient spreads its
    /// samples too thin for any colour bucket to hold 1 % of them).
    fn photo(x: usize, y: usize) -> [u8; 3] {
        let v = |n: usize| u8::try_from(n % 256).expect("a byte");
        let (bx, by) = (x / 8, y / 8);
        [v(bx * 32), v(by * 32), v((bx + by) * 16 + 40)]
    }

    #[test]
    fn a_colourful_picture_is_a_photo_and_keeps_its_colours() {
        let stats = image(64, 64, photo);
        assert!(stats.significant_buckets >= 16, "{stats:?}");
        assert!(stats.looks_like_photo());
        assert!(stats.keeps_colors(0.2));
    }

    #[test]
    fn a_flat_light_panel_follows_the_page() {
        let stats = image(64, 48, |_, _| [250, 244, 225]);
        assert_eq!(stats.high_lum_ratio, 1.0);
        assert!(!stats.looks_like_photo());
        assert!(!stats.keeps_colors(0.2));
    }

    #[test]
    fn artwork_on_a_white_backdrop_follows_the_page() {
        // A colourful centre on a flat white surround: the border makes it a backdrop (Enhanced #6088).
        let stats = image(64, 64, |x, y| {
            let inner = (16..48).contains(&x) && (16..48).contains(&y);
            if inner { photo(x, y) } else { [255, 255, 255] }
        });
        assert!(stats.border_uniformity >= 0.99 && stats.border_light_ratio >= 0.99);
        assert!(!stats.keeps_colors(0.2));
    }

    #[test]
    fn a_dark_painting_keeps_its_colours_even_filling_the_page() {
        let stats = image(64, 64, |x, y| {
            let v = |n: usize| u8::try_from(n % 90).expect("a byte");
            [v(x * 3), v(y * 2 + 10), v(x + y)]
        });
        assert!(stats.looks_like_dark_artwork(0.9), "{stats:?}");
        assert!(stats.keeps_colors(0.9));
        assert!(!page_dominant_recolors(Some(&stats), 0.9));
    }

    #[test]
    fn a_page_sized_scan_or_an_unsampled_picture_recolours() {
        let scan = image(64, 80, |x, y| {
            if (y % 8 == 3) && (x % 5 != 0) {
                [20, 20, 20]
            } else {
                [246, 244, 240]
            }
        });
        assert!(page_dominant_recolors(Some(&scan), 0.95));
        assert!(page_dominant_recolors(None, 0.8));
        assert!(
            !page_dominant_recolors(None, 0.5),
            "below 0.75 the other rules decide"
        );
    }

    #[test]
    fn nothing_to_measure_is_none() {
        assert_eq!(SampleStats::measure(&[], 0, 0, 0, 3), None);
    }
}
