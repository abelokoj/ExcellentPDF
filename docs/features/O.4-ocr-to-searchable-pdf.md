# Ph5: 40/45 | O.4: OCR to searchable PDF

**Tag:** O.4; **Priority:** P1; **Phase:** 5; **Needs:** K.21, K.22; **Code:** `ecclesia-features::ocr` (`ocr/model.rs`, `ocr/engine.rs`, `ocr/pdf_layer.rs`)
**Plan:** §27 entry `#### Ph5: 40/45 | O.4`.

## Problem

Scanned PDFs and image-only documents look crisp visually but cannot be searched, copied, or indexed for full-text retrieval. Re-rendering the entire page to an image destroys vector graphics and bloats file size.

## User story

As a reader, I want to run OCR on scanned PDFs so that every word becomes searchable and selectable while the original page appearance and vector graphics stay intact.

## UI sketch

- Triggered from the CLI (`ecclesia-cli ocr`), agent tools (`doc.ocr`), and background scanned-page detection (`detect_scanned_page`).
- Language packs (`standard_language_packs`, `discover_installed_packs`) list installed `.traineddata` packs alongside downloadable `ocr-<lang>` pack IDs.

## Mode

All modes; runs offline via local Tesseract CLI or Windows `Windows.Media.Ocr`, never sending document pixels over the network.

## Speed and size impact

- Startup: zero startup cost (`ecclesia-features::ocr` loads on demand).
- Installer size: core binary stays under budget; Tesseract `.traineddata` language files remain external packs.

## Design

- **Model (`ocr::model`)**: `PageImageInput`, `LanguagePack`, `PreprocessConfig` (`DeskewMode`, `BinarizeMode::Otsu`, `DenoiseMode::Median3x3`), `OcrConfig`, `WordBox`, `OcrPageResult`, and `detect_scanned_page`.
- **Engines (`ocr::engine`)**: Object-safe `OcrEngine` trait with `DeterministicTestEngine`, `TesseractCliEngine` (Netpbm `P5`/`P6` stdin + `quick-xml` hOCR parser), `WindowsOcrEngine` (PowerShell WinRT bridge on Windows), and `AutoOcrEngine`.
- **PDF writer (`ocr::pdf_layer`)**: `build_text_layer_plan` transforms image pixels (y-down) to PDF user space (y-up) and emits `q / BT / 3 Tr ... ET / Q` invisible text operators. `apply_text_layer_to_pdf_bytes` appends a vector-preserving incremental PDF update (supporting both classic `xref` tables and PDF 1.5+ `/XRef` streams).

## Acceptance tests

- [x] `otsu_separates_bimodal_image`, `discover_packs_marks_traineddata_files`: preprocessing and language pack discovery.
- [x] `hocr_parses_word_boxes_and_confidences`, `ppm_encodes_rgb_and_gray`: hOCR parsing and Netpbm encoding.
- [x] `text_layer_plan_emits_invisible_tr3_and_maps_y_up`: `3 Tr` operator generation and coordinate mapping.
- [x] `ocr_adds_searchable_text_to_vector_pdf_preserving_original_graphics`: incremental `Tr 3` overlay on `heavy-vector.pdf` is searchable via MuPDF while preserving original bytes.
- [x] `detect_scanned_distinguishes_text_pdf_from_non_text_pdf`: scanned-page heuristic on `base14-outline-annot.pdf`.
