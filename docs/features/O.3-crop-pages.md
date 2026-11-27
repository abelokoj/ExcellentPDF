# Ph7: 15/32 | O.3: Crop pages

**Tag:** O.3; **Priority:** P0; **Phase:** 7; **Needs:** O.1; **Code:** `ecclesia-core::page_ops` (`set_page_box`, `uncrop_pages`, `auto_crop_to_content`, `PdfBoxKind`, `PageTarget`)
**Plan:** §27 entry `#### Ph7: 15/32 | O.3`.

## Problem

Scanned books and wide-margin PDFs waste screen space and print paper; users need visual or automatic margin cropping that modifies `/CropBox`, `/TrimBox`, or `/BleedBox` reversibly across all, odd, even, or selected pages.

## User story

As a reader or prepress operator, I want to crop margins across selected/even/odd/all pages, auto-trim white borders to content bounds, and undo or uncrop cleanly back to `/MediaBox`.

## UI sketch

- Interactive crop rectangle overlay with `/CropBox`/`/TrimBox`/`/BleedBox` selector, `Auto-trim white margins` button, `All / Even / Odd / Range` apply scope, and `Remove crop`.

## Mode

Organize / Crop pages tool.

## Speed and size impact

- Box updates rewrite only targeted `/Page` dictionaries; `auto_crop_to_content` renders low-DPI analysis pixmaps for the selected pages (`detect_non_white_rect`).

## Design

- **`set_page_box` & `uncrop_pages`**: Clamps target rectangles to `/MediaBox`, writes `/CropBox`, `/TrimBox`, or `/BleedBox` (`PdfBoxKind`) across `PageTarget` (`All`, `Even`, `Odd`, `Range`, `Indices`), and restores original page bounds via `uncrop_pages`.
- **`auto_crop_to_content`**: Renders each page, scans non-white pixel bounding boxes with configurable padding, and commits `/CropBox` in PDF user-space coordinates.

## Acceptance tests

- [x] `crop_uncrop_and_auto_crop_corpus_file`: applies `/CropBox` on a corpus PDF, verifies reduced MuPDF `page.bounds()`, runs `uncrop_pages` to restore `/MediaBox`, and exercises `auto_crop_to_content` with odd/even page targeting.
