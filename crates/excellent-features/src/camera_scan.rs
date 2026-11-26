//! Document camera scanner pipeline (plan O.21): Sobel edge detection & quad contour candidates,
//! 4-corner projective homography & perspective warp, keystone crop, adaptive whiteboard/document
//! contrast cleanup, and multi-page searchable scanned PDF assembly via `excellent-core`/`excellent-mupdf`.

pub mod detect;
pub mod enhance;
pub mod pdf_builder;
pub mod types;
pub mod warp;

pub use detect::{QuadCandidate, SobelEdgeMap, detect_document_quads};
pub use enhance::{CleanupPreset, enhance_document};
pub use pdf_builder::{
    ScanPdfOptions, ScanSource, ScannedCaptureSession, ScannedPdfOutput, build_scanned_pdf,
};
pub use types::{
    CameraScanError, KeystoneTarget, Point2D, QuadCorners, RawImage, Result, ScanPixelFormat,
};
pub use warp::{Homography, compute_homography, perspective_warp};
