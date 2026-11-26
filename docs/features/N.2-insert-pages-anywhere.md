# Ph4: 15/44 | N.2: Insert pages anywhere

**Tag:** N.2; **Priority:** P0; **Phase:** 4; **Needs:** K.14; **Code:** `excellent-features::notebook::pages`, `excellent-core::notebook::pdf_ops`
**Plan:** §27 entry `#### Ph4: 15/44 | N.2`.

## Problem

Readers annotating dense PDFs or writing notebooks often run out of space mid-document and need to insert a blank or ruled page immediately before or after the current page without breaking the table of contents.

## User story

As a student taking notes in a textbook PDF, I want to insert a lined or Cornell page right after the current page so I can work out derivations without breaking bookmarks.

## UI sketch

- Entry points: Page menu ("Insert page before / after"), thumbnail context menu, command palette (`notebook.insert_page_before`, `notebook.insert_page_after`).
- Source choices: blank, procedural template (N.1), or copy current page size.

## Mode

Read, Annotate, and Write modes.

## Speed and size impact

- Startup: zero cost.
- Incremental PDF page tree update keeps saves fast and preserves existing bytes.

## Design

- `excellent_core::notebook::pdf_ops::insert_page` appends a new PDF page object and updates `/Pages /Kids` incrementally while preserving indirect page object identities so `/Outlines` destinations keep pointing to the right pages.
- `excellent_features::notebook::pages::{insert_page_in_pdf, insert_page_in_notebook}` support `InsertPosition::Before | After` and `InsertPageSource::{Blank, Template, CopyCurrentSize}`, shifting notebook sections, page metadata, and internal links consistently.

## Acceptance tests

- [x] `inserted_pages_show_in_mupdf_and_outline_points_to_right_pages`: inserts a Cornell template page before an existing chapter page, opens in MuPDF, verifies 3 pages and that the chapter outline entry now points to page index 2.
- [x] `insert_page_preserves_incremental_prefix_and_outline_targets`: verifies incremental PDF prefix preservation and outline accuracy.
