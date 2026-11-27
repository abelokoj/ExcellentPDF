# Ph7: 20/32 | O.9: Images to PDF and PDF to images

**Tag:** O.9; **Priority:** P1; **Phase:** 7; **Needs:** K.3; **Code:** `ecclesia-core::export_doc` (`natural_sort_paths`, `images_to_pdf`, `pdf_to_images`)
**Plan:** §27 entry `#### Ph7: 20/32 | O.9`.

## Problem

Users need to turn naturally ordered folders of scanned pages or photos (`page_1.png`, `page_2.png`, `page_10.png`) into a single PDF and export PDF page ranges back to crisp PNG or PPM images at selectable DPI.

## User story

As a user batch-converting scans or slides, I want folder image imports sorted naturally (`page_2` before `page_10`) into a clean multi-page PDF, and one-step PDF-to-PNG export at custom DPI.

## UI sketch

- "Images to PDF" importer showing naturally ordered file list + page sizing options (`FitImage`, `A4`, `Letter`).
- "Export as Images" dialog offering format (`Png`, `Ppm`), DPI selector, and page range.

## Mode

File menu (`Import Images as PDF`, `Export Pages as Images`).

## Speed and size impact

- Encodes raster inputs directly into Flate PDF `/Image` XObjects and renders pages to PNG/PPM buffers via MuPDF on demand.

## Design

- **`natural_sort_paths`**: Splits filenames into numeric and lexicographic runs so `img_2.png` precedes `img_10.png`.
- **`images_to_pdf` / `ImagePageInput`**: Embeds decoded RGB/RGBA buffers (`write_image_xobject`) with optional soft-mask alpha and page sizing (`ImagePageFit`).
- **`pdf_to_images`**: Renders selected pages at configured DPI to `ImageExportFormat::Png | Ppm`.

## Acceptance tests

- [x] `natural_sort_and_two_hundred_images_round_trip_to_pdf_and_png`: naturally sorts numbered paths, builds a 200-image PDF, reopens cleanly in MuPDF, and exports pages back to valid PNG bytes.
