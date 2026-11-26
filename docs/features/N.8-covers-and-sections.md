# Ph4: 21/44 | N.8: Covers and sections

**Tag:** N.8; **Priority:** P1; **Phase:** 4; **Needs:** N.4, B.1; **Code:** `excellent-features::notebook::sections`
**Plan:** §27 entry `#### Ph4: 21/44 | N.8`.

## Problem

Multi-topic notebooks become hard to navigate without visual cover identity, colored section dividers, and drag-and-drop section reordering.

## User story

As a student keeping a semester notebook, I want a cover page and colored section tabs I can drag to reorder, and I want those sections to export as PDF bookmarks.

## UI sketch

- Notebook cover picker (Classic, Linen, Blueprint, Image) and colored section tab strip beside the page; drag tabs to reorder sections.

## Mode

Write mode (and Library thumbnails).

## Speed and size impact

- Startup: zero cost.
- Reordering sections rearranges `/Pages /Kids` incrementally without rewriting page content streams.

## Design

- `set_cover`, `add_section`, `tab_strip` (normalized tab positions beside the page), `outline_entries`, and `reorder_section` (computes exact page permutation keeping cover anchored, rearranges PDF pages via `pdf_ops::rearrange_pages`, updates `/Outlines`, and remaps internal links).

## Acceptance tests

- [x] `sections_export_as_pdf_bookmarks_and_reorder_moves_pages`: sets a Linen cover, creates two sections, drags the second section before the first, exports to PDF, and verifies in MuPDF that both `/Outlines` bookmarks and actual page text moved to the new order.
