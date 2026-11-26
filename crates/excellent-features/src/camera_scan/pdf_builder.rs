//! Multi-page scanned PDF builder and stateful tablet capture session connecting O.21 camera
//! captures to O.9 (`images_to_pdf`) and O.4 (`ocr_pdf_bytes`).

use excellent_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};
use excellent_mupdf::Document as MuDocument;

use crate::ocr::{OcrConfig, OcrDocumentReport, OcrEngine, ocr_pdf_bytes};

use super::detect::detect_document_quads;
use super::enhance::{CleanupPreset, enhance_document};
use super::types::{CameraScanError, KeystoneTarget, QuadCorners, RawImage, Result};
use super::warp::perspective_warp;

/// Input source for one scanned page.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ScanSource {
    /// Raw camera capture requiring optional auto-quad detection and perspective warp.
    RawCameraFrame {
        /// Captured camera photo.
        image: RawImage,
        /// Explicit 4-corner boundary; if `None`, [`detect_document_quads`] picks the top candidate.
        quad: Option<QuadCorners>,
    },
    /// Already cropped and de-skewed page image from iPad VisionKit or Android ML Kit Document Scanner.
    PreCroppedScannerPage(RawImage),
}

/// Options governing multi-page scanned PDF creation.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanPdfOptions {
    /// Target PDF page geometry (`FitToImage`, `A4`, `Letter`, etc.).
    pub paper: PdfPaperSize,
    /// Margin in PDF points around each scanned page image.
    pub margin_pt: f32,
    /// Assumed capture DPI when `paper` is `PdfPaperSize::FitToImage`.
    pub dpi: f32,
    /// Adaptive contrast/background cleanup preset.
    pub cleanup: CleanupPreset,
    /// Keystone aspect ratio target for perspective rectification.
    pub keystone: KeystoneTarget,
    /// Optional OCR configuration used when an [`OcrEngine`] is supplied to [`build_scanned_pdf`].
    pub ocr_config: OcrConfig,
}

impl Default for ScanPdfOptions {
    fn default() -> Self {
        Self {
            paper: PdfPaperSize::A4,
            margin_pt: 0.0,
            dpi: 150.0,
            cleanup: CleanupPreset::DocumentColor,
            keystone: KeystoneTarget::AutoAspect,
            ocr_config: OcrConfig::default(),
        }
    }
}

/// Output of [`build_scanned_pdf`].
#[derive(Debug, Clone)]
pub struct ScannedPdfOutput {
    /// Assembled multi-page PDF bytes (searchable when OCR ran).
    pub pdf_bytes: Vec<u8>,
    /// Total pages in the built PDF.
    pub page_count: usize,
    /// Quadrilateral detected/applied for each raw camera page (`None` for pre-cropped pages).
    pub detected_quads: Vec<Option<QuadCorners>>,
    /// OCR report if an [`OcrEngine`] was provided.
    pub ocr_report: Option<OcrDocumentReport>,
}

/// Processes `pages` through quad detection, perspective warp, adaptive cleanup, `images_to_pdf`,
/// and optional O.4 OCR into a searchable multi-page PDF (plan O.21).
///
/// # Errors
///
/// Returns [`CameraScanError`] if `pages` is empty, warping fails, PDF creation fails, or OCR fails.
pub fn build_scanned_pdf(
    pages: &[ScanSource],
    options: &ScanPdfOptions,
    ocr_engine: Option<&dyn OcrEngine>,
) -> Result<ScannedPdfOutput> {
    if pages.is_empty() {
        return Err(CameraScanError::EmptySession);
    }

    let mut pdf_inputs = Vec::with_capacity(pages.len());
    let mut detected_quads = Vec::with_capacity(pages.len());

    for source in pages {
        let (rectified, used_quad) = match source {
            ScanSource::PreCroppedScannerPage(img) => (img.clone(), None),
            ScanSource::RawCameraFrame { image, quad } => {
                let chosen = match *quad {
                    Some(explicit) => Some(explicit),
                    None => detect_document_quads(image)?
                        .first()
                        .map(|cand| cand.corners),
                };
                match chosen {
                    Some(q) => (perspective_warp(image, q, options.keystone)?, Some(q)),
                    None => (image.clone(), None),
                }
            }
        };

        let cleaned = enhance_document(&rectified, options.cleanup)?;
        pdf_inputs.push(ImageInput::Rgb8 {
            width: cleaned.width(),
            height: cleaned.height(),
            dpi: options.dpi.max(72.0),
            samples: cleaned.to_rgb8_bytes(),
        });
        detected_quads.push(used_quad);
    }

    let base_pdf = images_to_pdf(&pdf_inputs, options.paper, options.margin_pt)?;
    let (final_pdf, ocr_report) = match ocr_engine {
        Some(engine) => {
            let (ocred_bytes, report) = ocr_pdf_bytes(&base_pdf, engine, &options.ocr_config)?;
            (ocred_bytes, Some(report))
        }
        None => (base_pdf, None),
    };

    // Verify the final PDF opens cleanly in MuPDF.
    let doc = MuDocument::from_bytes(&final_pdf, "scanned.pdf").map_err(|err| {
        CameraScanError::Pdf(excellent_core::page_ops::PdfDocError::InvalidPdf(
            err.to_string(),
        ))
    })?;
    let page_count = doc.page_count().map_err(|err| {
        CameraScanError::Pdf(excellent_core::page_ops::PdfDocError::InvalidPdf(
            err.to_string(),
        ))
    })?;

    Ok(ScannedPdfOutput {
        pdf_bytes: final_pdf,
        page_count,
        detected_quads,
        ocr_report,
    })
}

/// Stateful capture session collecting scanned pages from iPad VisionKit or Android ML Kit/CameraX.
#[derive(Debug, Clone, Default)]
pub struct ScannedCaptureSession {
    pages: Vec<ScanSource>,
}

impl ScannedCaptureSession {
    /// Creates an empty capture session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a raw camera capture (with optional manual corner override).
    pub fn add_camera_frame(&mut self, image: RawImage, quad: Option<QuadCorners>) {
        self.pages.push(ScanSource::RawCameraFrame { image, quad });
    }

    /// Appends a pre-cropped page returned by VisionKit (`VNDocumentCameraViewController`)
    /// or ML Kit Document Scanner.
    pub fn add_system_scanner_page(&mut self, image: RawImage) {
        self.pages.push(ScanSource::PreCroppedScannerPage(image));
    }

    /// Number of captured pages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Returns whether no pages have been captured yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Removes the page at `index` if present.
    pub fn remove_page(&mut self, index: usize) -> Option<ScanSource> {
        if index < self.pages.len() {
            Some(self.pages.remove(index))
        } else {
            None
        }
    }

    /// Builds the multi-page scanned PDF from all captured pages.
    ///
    /// # Errors
    ///
    /// Returns [`CameraScanError`] if the session is empty or PDF/OCR assembly fails.
    pub fn build_pdf(
        &self,
        options: &ScanPdfOptions,
        ocr_engine: Option<&dyn OcrEngine>,
    ) -> Result<ScannedPdfOutput> {
        build_scanned_pdf(&self.pages, options, ocr_engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_scan::types::{Point2D, ScanPixelFormat};
    use crate::ocr::{DeterministicTestEngine, WordBox};

    #[test]
    fn ten_page_scanned_document_is_built_and_searchable_in_mupdf() {
        let mut session = ScannedCaptureSession::new();
        let mut engine = DeterministicTestEngine::new(Vec::new());

        for page_idx in 0..10usize {
            let w = 80u32;
            let h = 100u32;
            let mut rgb = vec![24u8; (w * h * 3) as usize];
            let quad = QuadCorners {
                top_left: Point2D::new(10.0, 10.0),
                top_right: Point2D::new(70.0, 12.0),
                bottom_right: Point2D::new(72.0, 90.0),
                bottom_left: Point2D::new(8.0, 88.0),
            };
            for y in 12..88 {
                for x in 10..70 {
                    let idx = ((y * w + x) * 3) as usize;
                    rgb[idx] = 235;
                    rgb[idx + 1] = 235;
                    rgb[idx + 2] = 232;
                }
            }
            let img = RawImage::try_new(w, h, ScanPixelFormat::Rgb8, rgb).expect("valid frame");
            if page_idx % 2 == 0 {
                session.add_camera_frame(img, Some(quad));
            } else {
                session.add_system_scanner_page(img);
            }

            engine.set_page_words(
                page_idx,
                vec![WordBox {
                    text: format!("ScanPage{page_idx}"),
                    left_px: 10.0,
                    top_px: 20.0,
                    right_px: 60.0,
                    bottom_px: 34.0,
                    confidence: 97.0,
                    baseline_px: Some(31.0),
                }],
            );
        }

        assert_eq!(session.len(), 10);
        let options = ScanPdfOptions {
            paper: PdfPaperSize::Letter,
            cleanup: CleanupPreset::DocumentColor,
            ..ScanPdfOptions::default()
        };
        let output = session
            .build_pdf(&options, Some(&engine))
            .expect("build 10-page searchable scanned pdf");
        assert_eq!(output.page_count, 10);
        let report = output.ocr_report.expect("ocr report");
        assert_eq!(report.pages_ocred, 10);

        let doc = MuDocument::from_bytes(&output.pdf_bytes, "scan10.pdf").expect("reopen");
        assert_eq!(doc.page_count().expect("count"), 10);
        for idx in 0..10usize {
            let page = doc.load_page(idx).expect("page");
            let text = page.text(None).expect("text");
            let extracted = text.text().expect("text string");
            assert!(
                extracted.contains(&format!("ScanPage{idx}")),
                "page {idx} missing searchable OCR text: {extracted:?}"
            );
        }
    }
}
