# Ph5: 5/45 | R.29: DjVu, CHM and CBR

**Tag:** R.29; **Priority:** P2; **Phase:** 5; **Needs:** R.28; **Code:** `excellent-core::formats_extra`
**Plan:** §27 entry `#### Ph5: 5/45 | R.29: DjVu, CHM and CBR`.

## Problem

Readers with DjVu scans, Microsoft CHM help manuals, and CBR/CBZ comic archives cannot inspect or open them cleanly alongside MuPDF's built-in formats.

## User story

As a reader, I want DjVu, CHM, CBZ, and CBR files detected accurately and exposed through a unified document trait with page bounds, text, and outlines where supported.

## UI sketch

- File > Open recognizes `.djvu`, `.djv`, `.chm`, `.cbz`, and `.cbr` via `ExtraFormat::detect`.
- Outline sidebar displays `.hhc` sitemap TOCs for CHM and `NAVM` bookmarks for DjVu.

## Mode

All reading modes.

## Speed and size impact

- Startup: zero cost; parsed lazily on document open.
- Licence compliance: zero proprietary UnRAR compression algorithm re-implementation; CBR parses RAR4/RAR5 headers, reads uncompressed `STORE` entries directly, and delegates compressed entries to an external `RarExtractor` trait.

## Design

- **Model (`excellent-core::formats_extra`):**
  - `ExtraDocument` trait unifying `DjVuDocument`, `ChmDocument`, and `ComicArchive` (CBZ + CBR).
  - `DjVuDocument`: parses `AT&TFORM` `DJVU`/`DJVM` containers, `DIRM`, `INFO`, `NAVM`, and `TXTa` text chunks.
  - `ChmDocument`: parses `ITSF` / `ITSP` / `PMGL` directory chunks, `.hhc` sitemap outline, and section-0 HTML topics.
  - `ComicArchive`: reads CBZ via `zip` with natural page sorting and PNG/GIF dimensions, and CBR via RAR4/RAR5 container inspection + `RarExtractor` adapter.

## Acceptance tests

- [x] `formats_extra::tests::djvu_single_page_parses_info_and_txta`: verifies DjVu `AT&TFORM` bounds and `TXTa` extraction.
- [x] `formats_extra::tests::chm_itss_parses_entries_html_text_and_hhc_toc`: verifies ITSS directory parsing, `.hhc` outline, and HTML text extraction.
- [x] `formats_extra::tests::cbz_sorts_pages_naturally_and_reads_png_dimensions`: verifies natural page sorting and PNG dimensions.
- [x] `formats_extra::tests::cbr_reads_store_and_delegates_compressed_to_extractor`: verifies RAR4 `STORE` reading and `RarExtractor` delegation.
