# Ph7: 28/32 | O.17: Bates numbering

**Tag:** O.17; **Priority:** P2; **Phase:** 7; **Needs:** O.18; **Code:** `ecclesia-core::bates`
**Plan:** §27 entry `#### Ph7: 28/32 | O.17`.

## Problem

Legal discovery and compliance productions require sequential, zero-padded Bates identifiers stamped consistently across batches of multi-page PDFs with configurable prefixes, suffixes, page ranges, and removable OCG backing.

## User story

As a legal or records user preparing document productions, I want continuous Bates numbering across 20+ PDFs with custom prefix/suffix/padding and margin placement.

## UI sketch

- Batch Bates dialog accepting Prefix, Start Number, Zero Padding, Suffix, Corner/Margin Position (`TopLeft`..`BottomRight`), and ordered file list with live preview (`ACME-000042`).

## Mode

Organize / Legal Bates Numbering tool.

## Speed and size impact

- Reuses `headers::inject_ocg_overlays` to append stamped OCG layers cleanly across arbitrarily many PDFs in linear time.

## Design

- **`BatesConfig`, `BatesPosition`, `format_bates_number`**: Formats sequential identifiers with prefix, zero-padded counter, and suffix.
- **`apply_bates_to_pdf` / `apply_bates_batch`**: Applies Bates stamps across configured page ranges in single or multi-document batches, maintaining continuous numbering (`BatesBatchReport`) inside named OCG layers so stamps remain clean and removable.

## Acceptance tests

- [x] `bates_numbering_continues_sequentially_across_twenty_files`: numbers 20 PDFs sequentially, verifies continuous page counters (`0001`..total) and MuPDF text extraction at target positions.
