//! Pure data types, language packs, scanned-page detection, and image preprocessing for O.4 OCR.

use std::fmt;
use std::fs;
use std::path::Path;

use ecclesia_mupdf::{Document, MuError, PageRect, Pixmap};

/// Errors produced by OCR configuration, preprocessing, recognition, or PDF overlay writing.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum OcrError {
    /// No configured OCR backend is available on this machine.
    BackendUnavailable(String),
    /// A requested language pack is invalid or not installed.
    MissingLanguage(String),
    /// The input image or PDF was invalid or malformed.
    InvalidInput(String),
    /// An underlying MuPDF call failed.
    Mupdf(MuError),
    /// An I/O error occurred while reading inputs, invoking a backend, or writing output.
    Io(std::io::Error),
    /// Parsing hOCR XML or backend output failed.
    Parse(String),
}

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable(msg) => write!(f, "ocr backend unavailable: {msg}"),
            Self::MissingLanguage(lang) => write!(f, "missing ocr language pack: {lang}"),
            Self::InvalidInput(msg) => write!(f, "invalid ocr input: {msg}"),
            Self::Mupdf(err) => write!(f, "mupdf error during ocr: {err}"),
            Self::Io(err) => write!(f, "i/o error during ocr: {err}"),
            Self::Parse(msg) => write!(f, "failed to parse ocr output: {msg}"),
        }
    }
}

impl std::error::Error for OcrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mupdf(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::BackendUnavailable(_)
            | Self::MissingLanguage(_)
            | Self::InvalidInput(_)
            | Self::Parse(_) => None,
        }
    }
}

impl From<MuError> for OcrError {
    fn from(err: MuError) -> Self {
        Self::Mupdf(err)
    }
}

impl From<std::io::Error> for OcrError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Pixel channel layout of a [`PageImageInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit grayscale, 1 byte per pixel.
    Gray8,
    /// 24-bit RGB, 3 bytes per pixel.
    Rgb8,
    /// 32-bit RGBA, 4 bytes per pixel.
    Rgba8,
}

impl PixelFormat {
    /// Number of bytes per pixel for this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// A rendered PDF page image ready for preprocessing and OCR.
#[derive(Debug, Clone, PartialEq)]
pub struct PageImageInput {
    /// Page index in the source document (zero-based).
    pub page_index: usize,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Pixel layout of [`Self::pixels`].
    pub format: PixelFormat,
    /// Tightly packed row-major pixel bytes (`width * height * bpp`).
    pub pixels: Vec<u8>,
    /// Page width in PDF points (1/72 inch).
    pub page_width_pts: f32,
    /// Page height in PDF points (1/72 inch).
    pub page_height_pts: f32,
    /// Rendering resolution in dots per inch.
    pub dpi: u32,
}

impl PageImageInput {
    /// Builds a [`PageImageInput`] from a MuPDF [`Pixmap`] and its page bounds in points.
    ///
    /// # Errors
    ///
    /// Returns [`OcrError::InvalidInput`] if the pixmap dimensions or component count are unsupported.
    pub fn from_mupdf_pixmap(
        page_index: usize,
        pixmap: &Pixmap,
        bounds: PageRect,
        dpi: u32,
    ) -> Result<Self, OcrError> {
        let width = pixmap.width();
        let height = pixmap.height();
        let components = pixmap.components();
        let format = match components {
            1 => PixelFormat::Gray8,
            3 => PixelFormat::Rgb8,
            4 => PixelFormat::Rgba8,
            other => {
                return Err(OcrError::InvalidInput(format!(
                    "unsupported pixmap component count: {other}"
                )));
            }
        };
        let row_bytes = width as usize * format.bytes_per_pixel();
        let stride = pixmap.stride();
        let samples = pixmap.samples();
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        if stride == row_bytes {
            pixels.extend_from_slice(&samples[..row_bytes * height as usize]);
        } else {
            for row in 0..height as usize {
                let start = row * stride;
                let end = start + row_bytes;
                let slice = samples.get(start..end).ok_or_else(|| {
                    OcrError::InvalidInput("pixmap sample buffer shorter than expected".to_owned())
                })?;
                pixels.extend_from_slice(slice);
            }
        }
        Ok(Self {
            page_index,
            width,
            height,
            format,
            pixels,
            page_width_pts: bounds.width().max(1.0),
            page_height_pts: bounds.height().max(1.0),
            dpi: dpi.max(72),
        })
    }

    /// Converts the image to an 8-bit grayscale buffer of length `width * height`.
    #[must_use]
    pub fn to_luma(&self) -> Vec<u8> {
        let count = self.width as usize * self.height as usize;
        let mut out = Vec::with_capacity(count);
        match self.format {
            PixelFormat::Gray8 => {
                out.extend_from_slice(&self.pixels[..count.min(self.pixels.len())]);
                out.resize(count, 255);
            }
            PixelFormat::Rgb8 => {
                for chunk in self.pixels.as_chunks::<3>().0.iter().take(count) {
                    let y = (u32::from(chunk[0]) * 77
                        + u32::from(chunk[1]) * 150
                        + u32::from(chunk[2]) * 29)
                        >> 8;
                    out.push(y as u8);
                }
                out.resize(count, 255);
            }
            PixelFormat::Rgba8 => {
                for chunk in self.pixels.as_chunks::<4>().0.iter().take(count) {
                    let y = (u32::from(chunk[0]) * 77
                        + u32::from(chunk[1]) * 150
                        + u32::from(chunk[2]) * 29)
                        >> 8;
                    out.push(y as u8);
                }
                out.resize(count, 255);
            }
        }
        out
    }
}

/// Metadata describing an OCR language pack (Tesseract `.traineddata` or OS language tag).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguagePack {
    /// Short language code (Tesseract three-letter code such as `"eng"`, `"deu"`, `"jpn"`).
    pub code: String,
    /// BCP-47 tag for OS OCR engines (such as `"en-US"`, `"de-DE"`, `"ja-JP"`).
    pub bcp47: String,
    /// Human-readable language name.
    pub display_name: String,
    /// Downloadable pack identifier (for `ecclesia-packs`).
    pub pack_id: String,
    /// Whether this language is currently available locally.
    pub installed: bool,
}

/// Standard catalog of downloadable/built-in OCR language packs.
#[must_use]
pub fn standard_language_packs() -> Vec<LanguagePack> {
    let entries = [
        ("eng", "en-US", "English"),
        ("deu", "de-DE", "German"),
        ("fra", "fr-FR", "French"),
        ("spa", "es-ES", "Spanish"),
        ("ita", "it-IT", "Italian"),
        ("por", "pt-PT", "Portuguese"),
        ("nld", "nl-NL", "Dutch"),
        ("pol", "pl-PL", "Polish"),
        ("rus", "ru-RU", "Russian"),
        ("jpn", "ja-JP", "Japanese"),
        ("kor", "ko-KR", "Korean"),
        ("chi_sim", "zh-Hans", "Chinese (Simplified)"),
        ("chi_tra", "zh-Hant", "Chinese (Traditional)"),
        ("ara", "ar-SA", "Arabic"),
    ];
    entries
        .into_iter()
        .map(|(code, bcp47, display_name)| LanguagePack {
            code: code.to_owned(),
            bcp47: bcp47.to_owned(),
            display_name: display_name.to_owned(),
            pack_id: format!("ocr-{code}"),
            installed: false,
        })
        .collect()
}

/// Scans `tessdata_dir` for `*.traineddata` files and marks matching language packs as installed.
#[must_use]
pub fn discover_installed_packs(tessdata_dir: &Path) -> Vec<LanguagePack> {
    let mut packs = standard_language_packs();
    let Ok(entries) = fs::read_dir(tessdata_dir) else {
        return packs;
    };
    let mut extra_codes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("traineddata")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            if let Some(pack) = packs.iter_mut().find(|p| p.code == stem) {
                pack.installed = true;
            } else {
                extra_codes.push(stem.to_owned());
            }
        }
    }
    extra_codes.sort();
    for code in extra_codes {
        packs.push(LanguagePack {
            bcp47: code.clone(),
            display_name: code.clone(),
            pack_id: format!("ocr-{code}"),
            code,
            installed: true,
        });
    }
    packs
}

/// How page skew should be corrected before OCR runs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DeskewMode {
    /// Estimate skew angle from horizontal text line projection and rotate back if > 0.25 degrees.
    #[default]
    Auto,
    /// Do not deskew.
    Disabled,
    /// Rotate by an explicit angle in degrees before recognition.
    ForceAngle(f32),
}

/// How grayscale pixels should be binarized during preprocessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BinarizeMode {
    /// Keep grayscale/color values as-is.
    #[default]
    None,
    /// Global Otsu threshold into clean black-on-white grayscale.
    Otsu,
}

/// Noise filtering applied before recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DenoiseMode {
    /// No spatial denoising.
    #[default]
    None,
    /// 3x3 median filter on grayscale image.
    Median3x3,
}

/// Image preprocessing options applied prior to OCR.
#[derive(Debug, Clone, PartialEq)]
pub struct PreprocessConfig {
    /// Skew correction mode.
    pub deskew: DeskewMode,
    /// Binarization mode.
    pub binarize: BinarizeMode,
    /// Denoising mode.
    pub denoise: DenoiseMode,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            deskew: DeskewMode::Auto,
            binarize: BinarizeMode::None,
            denoise: DenoiseMode::None,
        }
    }
}

/// Whether pages that already contain selectable text should be skipped during batch OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExistingTextPolicy {
    /// Skip pages that already have selectable characters.
    #[default]
    SkipPagesWithText,
    /// Run OCR on every page regardless of existing text.
    ForceAllPages,
}

/// Full configuration for an OCR run.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrConfig {
    /// Ordered language codes (Tesseract style such as `"eng"`, `"deu"`).
    pub languages: Vec<String>,
    /// Rendering DPI for pages sent to OCR.
    pub dpi: u32,
    /// Preprocessing steps run on the rendered page image.
    pub preprocess: PreprocessConfig,
    /// Whether to skip pages that already contain selectable text.
    pub existing_text: ExistingTextPolicy,
    /// Optional path to a directory holding `.traineddata` language packs.
    pub tessdata_dir: Option<std::path::PathBuf>,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            languages: vec!["eng".to_owned()],
            dpi: 300,
            preprocess: PreprocessConfig::default(),
            existing_text: ExistingTextPolicy::SkipPagesWithText,
            tessdata_dir: None,
        }
    }
}

/// A single recognized word and its pixel-space bounding box on the rendered image.
#[derive(Debug, Clone, PartialEq)]
pub struct WordBox {
    /// Recognized word text.
    pub text: String,
    /// Left edge in image pixels.
    pub left_px: f32,
    /// Top edge in image pixels.
    pub top_px: f32,
    /// Right edge in image pixels.
    pub right_px: f32,
    /// Bottom edge in image pixels.
    pub bottom_px: f32,
    /// Engine confidence in `0.0..=100.0`.
    pub confidence: f32,
    /// Optional baseline y-coordinate in image pixels.
    pub baseline_px: Option<f32>,
}

/// Complete OCR result for one page.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrPageResult {
    /// Zero-based page index.
    pub page_index: usize,
    /// Width of the image passed to OCR, in pixels.
    pub image_width: u32,
    /// Height of the image passed to OCR, in pixels.
    pub image_height: u32,
    /// Recognized words in reading order.
    pub words: Vec<WordBox>,
    /// Joined plain text of all words.
    pub full_text: String,
    /// Mean confidence across all recognized words (`0.0` if no words).
    pub mean_confidence: f32,
    /// Detected or applied deskew angle in degrees.
    pub skew_degrees: f32,
    /// Identifier of the engine that produced this result.
    pub engine_name: String,
}

impl OcrPageResult {
    /// Builds an [`OcrPageResult`] from recognized words, computing `full_text` and `mean_confidence`.
    #[must_use]
    pub fn from_words(
        page_index: usize,
        image_width: u32,
        image_height: u32,
        words: Vec<WordBox>,
        skew_degrees: f32,
        engine_name: impl Into<String>,
    ) -> Self {
        let full_text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mean_confidence = if words.is_empty() {
            0.0
        } else {
            words.iter().map(|w| w.confidence).sum::<f32>() / words.len() as f32
        };
        Self {
            page_index,
            image_width,
            image_height,
            words,
            full_text,
            mean_confidence,
            skew_degrees,
            engine_name: engine_name.into(),
        }
    }
}

/// Classification of whether a PDF page appears to be scanned/image-only or already has text.
#[derive(Debug, Clone, PartialEq)]
pub enum PageScanStatus {
    /// Page already contains selectable text characters.
    HasSelectableText {
        /// Number of non-whitespace characters extracted from the page.
        char_count: usize,
    },
    /// Page has little or no selectable text and draws large raster image(s).
    LikelyScanned {
        /// Number of images drawn on the page.
        image_count: usize,
        /// Fraction of the page area covered by images (`0.0..=1.0`).
        image_coverage: f32,
    },
    /// Page has neither selectable text nor raster images (e.g., blank or pure vector).
    Empty,
}

/// Inspects `page_index` of `document` to determine whether it needs OCR.
///
/// # Errors
///
/// Returns [`OcrError::Mupdf`] if the page cannot be loaded or inspected.
pub fn detect_scanned_page(
    document: &Document,
    page_index: usize,
) -> Result<PageScanStatus, OcrError> {
    let page = document.load_page(page_index)?;
    let layout = page.text(None)?.layout();
    let char_count = layout
        .chars
        .iter()
        .filter(|ch| !ch.c.is_whitespace())
        .count();
    if char_count > 0 {
        return Ok(PageScanStatus::HasSelectableText { char_count });
    }
    let bounds = page.bounds()?;
    let page_area = (bounds.width().max(1.0)) * (bounds.height().max(1.0));
    let list = page.display_list(None)?;
    let images = list.images(None)?;
    if images.is_empty() {
        return Ok(PageScanStatus::Empty);
    }
    let covered: f32 = images
        .iter()
        .map(|img| {
            let b = img.bounds();
            b.width().max(0.0) * b.height().max(0.0)
        })
        .sum();
    Ok(PageScanStatus::LikelyScanned {
        image_count: images.len(),
        image_coverage: (covered / page_area).clamp(0.0, 1.0),
    })
}

/// Applies preprocessing (`denoise`, `binarize`, `deskew`) to `input` and returns the processed image
/// plus the applied skew angle in degrees.
#[must_use]
pub fn preprocess_image(
    input: &PageImageInput,
    config: &PreprocessConfig,
) -> (PageImageInput, f32) {
    let needs_processing = !matches!(config.deskew, DeskewMode::Disabled)
        || config.binarize != BinarizeMode::None
        || config.denoise != DenoiseMode::None;
    if !needs_processing || input.width == 0 || input.height == 0 {
        return (input.clone(), 0.0);
    }

    let mut luma = input.to_luma();
    let w = input.width as usize;
    let h = input.height as usize;

    if config.denoise == DenoiseMode::Median3x3 && w >= 3 && h >= 3 {
        luma = median_3x3(&luma, w, h);
    }

    if config.binarize == BinarizeMode::Otsu {
        let thresh = otsu_threshold(&luma);
        for px in &mut luma {
            *px = if *px >= thresh { 255 } else { 0 };
        }
    }

    let skew_degrees = match config.deskew {
        DeskewMode::Disabled => 0.0,
        DeskewMode::ForceAngle(angle) => angle,
        DeskewMode::Auto => estimate_skew_degrees(&luma, w, h),
    };

    if skew_degrees.abs() >= 0.25 {
        luma = rotate_luma_nearest(&luma, w, h, -skew_degrees);
    }

    let processed = PageImageInput {
        page_index: input.page_index,
        width: input.width,
        height: input.height,
        format: PixelFormat::Gray8,
        pixels: luma,
        page_width_pts: input.page_width_pts,
        page_height_pts: input.page_height_pts,
        dpi: input.dpi,
    };
    (processed, skew_degrees)
}

fn median_3x3(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut dst = src.to_vec();
    let mut win = [0u8; 9];
    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let mut k = 0;
            for dy in [-1isize, 0, 1] {
                for dx in [-1isize, 0, 1] {
                    let ny = (y as isize + dy) as usize;
                    let nx = (x as isize + dx) as usize;
                    win[k] = src[ny * w + nx];
                    k += 1;
                }
            }
            win.sort_unstable();
            dst[y * w + x] = win[4];
        }
    }
    dst
}

fn otsu_threshold(luma: &[u8]) -> u8 {
    if luma.is_empty() {
        return 128;
    }
    let mut hist = [0u64; 256];
    for &px in luma {
        hist[usize::from(px)] += 1;
    }
    let total = luma.len() as f64;
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| (i as f64) * (c as f64))
        .sum();
    let mut sum_bg = 0.0;
    let mut weight_bg = 0.0;
    let mut max_var = -1.0;
    let mut best_sum = 0usize;
    let mut best_count = 0usize;
    for (t, &count) in hist.iter().enumerate() {
        weight_bg += count as f64;
        if weight_bg == 0.0 {
            continue;
        }
        let weight_fg = total - weight_bg;
        if weight_fg == 0.0 {
            break;
        }
        sum_bg += (t as f64) * (count as f64);
        let mean_bg = sum_bg / weight_bg;
        let mean_fg = (sum_all - sum_bg) / weight_fg;
        let diff = mean_bg - mean_fg;
        let var = weight_bg * weight_fg * diff * diff;
        if var > max_var + 1e-6 {
            max_var = var;
            best_sum = t;
            best_count = 1;
        } else if (var - max_var).abs() <= 1e-6 {
            best_sum += t;
            best_count += 1;
        }
    }
    best_sum.checked_div(best_count).map_or(128, |v| v as u8)
}

/// Estimates small text skew (`-5.0..=5.0` degrees) by maximizing variance of horizontal dark-pixel projections.
fn estimate_skew_degrees(luma: &[u8], w: usize, h: usize) -> f32 {
    if w < 16 || h < 16 {
        return 0.0;
    }
    // Downsample to keep projection evaluation fast on high-DPI pages.
    let step = (w.max(h) / 256).max(1);
    let sw = w / step;
    let sh = h / step;
    if sw < 8 || sh < 8 {
        return 0.0;
    }
    let mut dark = vec![0.0f32; sw * sh];
    let mut dark_count = 0usize;
    for y in 0..sh {
        for x in 0..sw {
            let px = luma[(y * step) * w + (x * step)];
            if px < 180 {
                dark[y * sw + x] = 1.0;
                dark_count += 1;
            }
        }
    }
    if dark_count < 8 {
        return 0.0;
    }
    let mut best_angle = 0.0f32;
    let mut best_score = -1.0f64;
    // Test angles from -5.0 to +5.0 degrees in 0.5-degree increments (0.0 tested first to break ties).
    for half_deg in -10..=10 {
        let angle = (half_deg as f32) * 0.5;
        let tan = angle.to_radians().tan();
        let mut row_sums = vec![0.0f64; sh];
        for y in 0..sh {
            for x in 0..sw {
                let val = dark[y * sw + x];
                if val > 0.0 {
                    let shifted_y =
                        (y as f32 - (x as f32 - sw as f32 * 0.5) * tan).round() as isize;
                    if shifted_y >= 0 && (shifted_y as usize) < sh {
                        row_sums[shifted_y as usize] += f64::from(val);
                    }
                }
            }
        }
        let score: f64 = row_sums.iter().map(|&s| s * s).sum();
        // Prefer 0.0 unless another angle beats it meaningfully (> 1%).
        if (half_deg == 0 && best_score < 0.0) || score > best_score * 1.01 {
            best_score = score;
            best_angle = angle;
        }
    }
    best_angle
}

fn rotate_luma_nearest(src: &[u8], w: usize, h: usize, angle_deg: f32) -> Vec<u8> {
    let rad = angle_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let cx = (w as f32) * 0.5;
    let cy = (h as f32) * 0.5;
    let mut dst = vec![255u8; w * h];
    for y in 0..h {
        let dy = y as f32 - cy;
        for x in 0..w {
            let dx = x as f32 - cx;
            let sx = (dx * cos + dy * sin + cx).round() as isize;
            let sy = (-dx * sin + dy * cos + cy).round() as isize;
            if sx >= 0 && (sx as usize) < w && sy >= 0 && (sy as usize) < h {
                dst[y * w + x] = src[sy as usize * w + sx as usize];
            }
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_separates_bimodal_image() {
        let mut pixels = vec![30u8; 64];
        pixels.extend(vec![220u8; 64]);
        let input = PageImageInput {
            page_index: 0,
            width: 16,
            height: 8,
            format: PixelFormat::Gray8,
            pixels,
            page_width_pts: 72.0,
            page_height_pts: 36.0,
            dpi: 72,
        };
        let (out, skew) = preprocess_image(
            &input,
            &PreprocessConfig {
                deskew: DeskewMode::Disabled,
                binarize: BinarizeMode::Otsu,
                denoise: DenoiseMode::None,
            },
        );
        assert_eq!(skew, 0.0);
        assert!(out.pixels[..64].iter().all(|&p| p == 0));
        assert!(out.pixels[64..].iter().all(|&p| p == 255));
    }

    #[test]
    fn discover_packs_marks_traineddata_files() {
        let dir = std::env::temp_dir().join(format!("ecclesia-ocr-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create pack dir");
        fs::write(dir.join("eng.traineddata"), b"stub").expect("write eng");
        fs::write(dir.join("custom.traineddata"), b"stub").expect("write custom");

        let packs = discover_installed_packs(&dir);
        let _ = fs::remove_dir_all(&dir);

        let eng = packs.iter().find(|p| p.code == "eng").expect("eng pack");
        assert!(eng.installed);
        let deu = packs.iter().find(|p| p.code == "deu").expect("deu pack");
        assert!(!deu.installed);
        let custom = packs
            .iter()
            .find(|p| p.code == "custom")
            .expect("custom pack");
        assert!(custom.installed);
    }
}
