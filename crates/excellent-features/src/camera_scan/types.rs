//! Core image, geometry, quad, and error types for the O.21 document camera scanner.

use std::fmt;

use excellent_core::page_ops::PdfDocError;

use crate::ocr::OcrError;

/// Errors produced by the document camera scanner pipeline.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum CameraScanError {
    /// Image dimensions or pixel buffer length were invalid.
    InvalidImage(String),
    /// A quadrilateral was degenerate, concave, self-intersecting, or too small.
    DegenerateQuad(String),
    /// No pages were provided to build a scanned PDF.
    EmptySession,
    /// Underlying PDF assembly or MuPDF verification failed.
    Pdf(PdfDocError),
    /// Underlying OCR pass failed.
    Ocr(OcrError),
}

impl fmt::Display for CameraScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidImage(msg) => write!(f, "invalid camera scan image: {msg}"),
            Self::DegenerateQuad(msg) => write!(f, "degenerate document quad: {msg}"),
            Self::EmptySession => write!(f, "camera scan session contains no pages"),
            Self::Pdf(err) => write!(f, "scanned pdf assembly failed: {err}"),
            Self::Ocr(err) => write!(f, "scanned pdf ocr failed: {err}"),
        }
    }
}

impl std::error::Error for CameraScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pdf(err) => Some(err),
            Self::Ocr(err) => Some(err),
            Self::InvalidImage(_) | Self::DegenerateQuad(_) | Self::EmptySession => None,
        }
    }
}

impl From<PdfDocError> for CameraScanError {
    fn from(err: PdfDocError) -> Self {
        Self::Pdf(err)
    }
}

impl From<OcrError> for CameraScanError {
    fn from(err: OcrError) -> Self {
        Self::Ocr(err)
    }
}

/// Convenience result alias for camera scanner operations.
pub type Result<T> = std::result::Result<T, CameraScanError>;

/// Pixel layout of a [`RawImage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScanPixelFormat {
    /// 8-bit grayscale (`1` byte per pixel).
    Gray8,
    /// 24-bit interleaved RGB (`3` bytes per pixel).
    Rgb8,
    /// 32-bit interleaved RGBA (`4` bytes per pixel).
    Rgba8,
}

impl ScanPixelFormat {
    /// Number of bytes per pixel.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// Owned, tightly packed camera image buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImage {
    width: u32,
    height: u32,
    format: ScanPixelFormat,
    data: Vec<u8>,
}

impl RawImage {
    /// Validates and constructs a [`RawImage`].
    ///
    /// # Errors
    ///
    /// Returns [`CameraScanError::InvalidImage`] if `width == 0`, `height == 0`, or `data.len()`
    /// does not equal `width * height * format.bytes_per_pixel()`.
    pub fn try_new(
        width: u32,
        height: u32,
        format: ScanPixelFormat,
        data: Vec<u8>,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(CameraScanError::InvalidImage(
                "image width and height must be positive".to_owned(),
            ));
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(format.bytes_per_pixel()))
            .ok_or_else(|| {
                CameraScanError::InvalidImage("image byte count overflows".to_owned())
            })?;
        if data.len() != expected {
            return Err(CameraScanError::InvalidImage(format!(
                "expected {expected} bytes for {width}x{height} {format:?}, got {}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            format,
            data,
        })
    }

    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Pixel format.
    #[must_use]
    pub const fn format(&self) -> ScanPixelFormat {
        self.format
    }

    /// Raw packed pixel slice.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Reads the RGB triplet at `(x, y)` (clamped to bounds).
    #[must_use]
    pub fn pixel_rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let cx = x.min(self.width - 1) as usize;
        let cy = y.min(self.height - 1) as usize;
        let bpp = self.format.bytes_per_pixel();
        let idx = (cy * self.width as usize + cx) * bpp;
        match self.format {
            ScanPixelFormat::Gray8 => {
                let v = self.data[idx];
                [v, v, v]
            }
            ScanPixelFormat::Rgb8 | ScanPixelFormat::Rgba8 => {
                [self.data[idx], self.data[idx + 1], self.data[idx + 2]]
            }
        }
    }

    /// Converts the image to 8-bit Rec.601 luminance (`width * height` bytes).
    #[must_use]
    pub fn to_luma(&self) -> Vec<u8> {
        let count = (self.width as usize) * (self.height as usize);
        let mut luma = Vec::with_capacity(count);
        match self.format {
            ScanPixelFormat::Gray8 => luma.extend_from_slice(&self.data),
            ScanPixelFormat::Rgb8 => {
                for px in self.data.as_chunks::<3>().0 {
                    let y =
                        (77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2]))
                            >> 8;
                    luma.push(y as u8);
                }
            }
            ScanPixelFormat::Rgba8 => {
                for px in self.data.as_chunks::<4>().0 {
                    let y =
                        (77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2]))
                            >> 8;
                    luma.push(y as u8);
                }
            }
        }
        luma
    }

    /// Converts the image to tightly packed `Rgb8` bytes.
    #[must_use]
    pub fn to_rgb8_bytes(&self) -> Vec<u8> {
        let count = (self.width as usize) * (self.height as usize);
        match self.format {
            ScanPixelFormat::Rgb8 => self.data.clone(),
            ScanPixelFormat::Gray8 => {
                let mut out = Vec::with_capacity(count * 3);
                for &v in &self.data {
                    out.extend_from_slice(&[v, v, v]);
                }
                out
            }
            ScanPixelFormat::Rgba8 => {
                let mut out = Vec::with_capacity(count * 3);
                for px in self.data.as_chunks::<4>().0 {
                    out.extend_from_slice(&px[..3]);
                }
                out
            }
        }
    }
}

/// 2D point in image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point2D {
    /// Horizontal coordinate in pixels.
    pub x: f64,
    /// Vertical coordinate in pixels.
    pub y: f64,
}

impl Point2D {
    /// Creates a point at `(x, y)`.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Euclidean distance to `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// Ordered 4-corner document boundary in image pixel space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadCorners {
    /// Top-left corner.
    pub top_left: Point2D,
    /// Top-right corner.
    pub top_right: Point2D,
    /// Bottom-right corner.
    pub bottom_right: Point2D,
    /// Bottom-left corner.
    pub bottom_left: Point2D,
}

impl QuadCorners {
    /// Orders four arbitrary points into canonical `(top_left, top_right, bottom_right, bottom_left)`.
    #[must_use]
    pub fn from_unordered(pts: [Point2D; 4]) -> Self {
        let mut by_sum = pts;
        by_sum.sort_by(|a, b| (a.x + a.y).total_cmp(&(b.x + b.y)));
        let top_left = by_sum[0];
        let bottom_right = by_sum[3];

        let mut mid = [by_sum[1], by_sum[2]];
        mid.sort_by(|a, b| (a.x - a.y).total_cmp(&(b.x - b.y)));
        let bottom_left = mid[0];
        let top_right = mid[1];

        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    /// Returns the corners in clockwise order `[tl, tr, br, bl]`.
    #[must_use]
    pub const fn as_array(self) -> [Point2D; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    /// Computes signed shoelace area (positive when ordered clockwise in top-down Y coordinates).
    #[must_use]
    pub fn area(self) -> f64 {
        let p = self.as_array();
        let mut sum = 0.0;
        for i in 0..4 {
            let j = (i + 1) % 4;
            sum += p[i].x * p[j].y - p[j].x * p[i].y;
        }
        0.5 * sum.abs()
    }

    /// Returns whether all four corners form a strictly convex non-degenerate quadrilateral.
    #[must_use]
    pub fn is_convex(self) -> bool {
        let p = self.as_array();
        let mut sign = 0.0_f64;
        for i in 0..4 {
            let a = p[i];
            let b = p[(i + 1) % 4];
            let c = p[(i + 2) % 4];
            let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
            if cross.abs() < 1e-6 {
                return false;
            }
            if sign == 0.0 {
                sign = cross;
            } else if sign * cross <= 0.0 {
                return false;
            }
        }
        true
    }

    /// Estimates natural rectified `(width, height)` in pixels from opposite edge lengths.
    #[must_use]
    pub fn natural_dimensions(self) -> (u32, u32) {
        let top = self.top_left.distance(self.top_right);
        let bottom = self.bottom_left.distance(self.bottom_right);
        let left = self.top_left.distance(self.bottom_left);
        let right = self.top_right.distance(self.bottom_right);
        let w = top.max(bottom).round().max(1.0) as u32;
        let h = left.max(right).round().max(1.0) as u32;
        (w, h)
    }
}

/// Target output aspect / dimensions for keystone crop & perspective rectification.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub enum KeystoneTarget {
    /// Infer rectified dimensions directly from the quad's longest horizontal and vertical edges.
    #[default]
    AutoAspect,
    /// Fit rectified output to ISO A4 aspect (`1 : sqrt(2)`).
    A4Portrait,
    /// Fit rectified output to US Letter aspect (`8.5 : 11`).
    LetterPortrait,
    /// Explicit pixel output dimensions `(width, height)`.
    ExplicitPixels {
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
    },
}
