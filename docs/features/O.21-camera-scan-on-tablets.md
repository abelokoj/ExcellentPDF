# Ph8: 3/3 | O.21: Camera scan on tablets

**Tag:** O.21; **Priority:** P2; **Phase:** 8; **Needs:** O.9, O.4; **Code:** `ecclesia-features::camera_scan` (`types`, `detect`, `warp`, `enhance`, `pdf_builder`), `android/.../CameraScanBridge.kt`
**Plan:** §27 entry `#### Ph8: 3/3 | O.21`.

## Problem

Tablet users photographing printed pages or whiteboards need automatic document border localization, perspective keystone correction, uneven shadow/contrast cleanup, and one-step assembly into a searchable multi-page PDF.

## User story

As a student or researcher scanning handouts and whiteboards on a tablet, I want photographed pages cropped, de-skewed, cleaned up, and saved as a multi-page searchable PDF.

## UI sketch

- Scan session tray accepting raw camera captures (with adjustable 4-corner handles) or pre-cropped system scanner pages (iPad VisionKit / Android ML Kit Document Scanner).
- Cleanup preset picker (`DocumentColor`, `Whiteboard`, `Grayscale`, `BlackAndWhite`, `Original`) and OCR toggle.

## Mode

Home / File import ("Scan from Camera") and Notebook insert.

## Speed and size impact

- Startup: zero startup cost; scanner modules load only when a scan session runs.
- Budgets: integral-image (`SummedAreaTable`) local statistics run in `O(W * H)` per page; 8×8 DLT homography solve runs in microseconds.
- Size: pure Rust implementation using existing `ecclesia-core` (`O.9`), `ecclesia-mupdf`, and `ecclesia-features::ocr` (`O.4`).

## Design

- **`detect`**: 3×3 Gaussian smoothing + Sobel gradient magnitude + adaptive Otsu thresholding + monotone-chain convex hull + area-preserving 4-corner polygon reduction (`detect_document_quads`).
- **`warp`**: 8×8 Gaussian-elimination DLT projective homography solver (`compute_homography`) + bilinear-interpolated `perspective_warp` with `KeystoneTarget::{AutoAspect, A4Portrait, LetterPortrait, ExplicitPixels}`.
- **`enhance`**: integral-image local illumination normalization (`DocumentColor`, `Whiteboard`, `Grayscale`) and Sauvola adaptive binarization (`BlackAndWhite`).
- **`pdf_builder`**: `ScannedCaptureSession` and `build_scanned_pdf` assembling multi-page PDFs via `ecclesia_core::export_doc::images_to_pdf`, verifying with `ecclesia_mupdf::Document`, and adding invisible OCR text layers (`3 Tr`) via `ecclesia_features::ocr::ocr_pdf_bytes`.

## Acceptance tests

- [x] `detects_skewed_bright_page_quad_on_dark_desk_background`: Sobel + hull quad detector localizes a perspective-skewed document quad.
- [x] `homography_maps_corners_and_rectifies_skewed_quad`: 4-corner projective homography and bilinear perspective warp produce a rectified page.
- [x] `whiteboard_cleanup_lifts_shaded_background_and_preserves_dark_strokes`: adaptive cleanup normalizes uneven lighting and Sauvola binarizes cleanly.
- [x] `ten_page_scanned_document_is_built_and_searchable_in_mupdf`: 10-page capture session builds a valid PDF with O.4 OCR text verified searchable in MuPDF on all 10 pages.
