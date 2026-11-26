# Ph7: 19/32 | O.8: Compress

**Tag:** O.8; **Priority:** P1; **Phase:** 7; **Needs:** K.15; **Code:** `excellent-core::compress`
**Plan:** §27 entry `#### Ph7: 19/32 | O.8`.

## Problem

Scanned or CAD/image-heavy PDFs frequently ship with uncompressed streams, redundant duplicate XObjects/fonts, and oversized 300+ DPI images that exceed email or portal upload limits.

## User story

As a user sharing PDFs, I want one-click compression presets (`Screen` 72 DPI, `Ebook` 150 DPI, `Print` 300 DPI) plus custom targets with an exact before/after size comparison.

## UI sketch

- Compress modal showing profile selector (`Screen`, `Ebook`, `Print`), optional stream deflation / object deduplication toggles, and live before vs. after byte savings summary.

## Mode

File / Optimize / Compress dialog.

## Speed and size impact

- Single linear pass hashing streams via SHA-256 for deduplication, downsampling high-DPI raster XObjects cleanly, and Flate-compressing uncompressed streams (`rewrite_clean`).

## Design

- **`CompressionProfile` & `CompressionSettings`**: Presets for `Screen`, `Ebook`, and `Print` controlling max image DPI, JPEG/Flate strategy, object deduplication (`deduplicate_objects`), and stream deflation (`deflate_streams`).
- **`compress_pdf` & `CompressionStats`**: Performs bilinear RGB/grayscale image downsampling (`maybe_downsample_image`), SHA-256 content-addressed object deduplication (`rewrite_refs`), Flate stream recompression, and non-incremental clean rewrite returning `CompressionStats` (`original_bytes`, `compressed_bytes`, `deduplicated_objects`, `downsampled_images`).

## Acceptance tests

- [x] `compression_deflates_deduplicates_and_downsamples_cleanly`: verifies Flate compression, duplicate object deduplication, and high-DPI image downsampling reduce byte size while remaining valid in MuPDF.
