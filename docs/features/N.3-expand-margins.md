# Ph4: 16/44 | N.3: Expand margins

**Tag:** N.3; **Priority:** P0; **Phase:** 4; **Needs:** K.14; **Code:** `excellent-features::notebook::pages`, `excellent-core::notebook::pdf_ops`
**Plan:** §27 entry `#### Ph4: 16/44 | N.3`.

## Problem

Academic papers and textbooks have narrow margins that leave no room for handwritten derivations or margin notes beside the text.

## User story

As a researcher reading a two-column paper, I want to expand the page's right margin so I can write notes beside the text that stay visible in Acrobat, and remove the added margins later restoring the exact original page boxes.

## UI sketch

- Entry points: Page menu ("Expand margins...", "Remove added margins"), toolbar action, command palette (`page.expand_margins`, `page.remove_margins`).
- Dialog offers left/right/top/bottom expansion in points or inches.

## Mode

Annotate and Write modes.

## Speed and size impact

- Startup: zero cost.
- Only updates the page dictionary's `/MediaBox`, `/CropBox`, and `/ExcellentPdfOrigBoxes` via incremental PDF update.

## Design

- `expand_page_margins_pdf` / `expand_page_margins_notebook` widen `/MediaBox` and `/CropBox` in PDF user space without shifting existing page content, storing the first pre-expansion `[media_box, crop_box]` in `/ExcellentPdfOrigBoxes` (and `notebook.json`).
- Repeated expansions never overwrite the original boxes.
- `remove_added_margins_pdf` / `remove_added_margins_notebook` restore the exact original `/MediaBox` and `/CropBox` and delete `/ExcellentPdfOrigBoxes`.

## Acceptance tests

- [x] `expand_margins_allows_margin_notes_and_remove_restores_exact_boxes`: widens a Letter page by 200 pt on the right and 20 pt on top, places a PDF note annotation in the expanded margin verified by MuPDF, then removes added margins and verifies exact restoration of MediaBox and CropBox.
- [x] `expand_and_remove_margins_restores_exact_boxes`: verifies multiple successive expansions still restore the original pre-expansion boxes exactly.
