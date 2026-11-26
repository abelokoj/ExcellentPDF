# Ph7: 14/32 | O.2: Split and merge

**Tag:** O.2; **Priority:** P0; **Phase:** 7; **Needs:** O.1; **Code:** `excellent-core::page_ops` (`merge_pdfs`, `split_pdf`, `SplitStrategy`, `OutlineSynthesis`)
**Plan:** §27 entry `#### Ph7: 14/32 | O.2`.

## Problem

Combining multiple PDFs or breaking large documents apart by page ranges, fixed intervals, top-level bookmarks, or target file size often discards bookmarks, internal links, or page resources.

## User story

As a user assembling or dividing documents, I want to merge dozens of PDFs with preserved or synthesized outlines and split any PDF by ranges, every-N pages, bookmarks, or max byte size.

## UI sketch

- Merge dialog accepting ordered PDFs + outline synthesis toggle (`Preserve` / `FromFileTitles` / `Omit`) and Split dialog offering `ByRanges`, `EveryN`, `ByOutlineLevel`, and `ByMaxBytes`.

## Mode

Organize mode (`Split` / `Merge`).

## Speed and size impact

- `merge_pdfs` streams objects through `RawPdfWriter` with object ID remapping and `/Pages` tree consolidation, scaling smoothly across 100+ input files.

## Design

- **`merge_pdfs(sources, synthesis)`**: Deep-clones page dictionaries and resource graphs (`clone_val_into`) into one `/Pages` root while preserving internal links and merging or synthesizing `/Outlines` trees per `OutlineSynthesis`.
- **`split_pdf(pdf_bytes, strategy)`**: Partitions pages into minimal clean PDFs by explicit `PageRange` spans, fixed chunk size (`EveryN`), outline depth boundaries (`ByOutlineLevel`), or greedy target byte size (`ByMaxBytes`).

## Acceptance tests

- [x] `merge_hundred_files_preserves_outlines_and_links_and_split_works`: merges 100 PDFs, verifies outline synthesis/preservation and links, then splits by ranges, every-N, outline level, and max byte budget.
