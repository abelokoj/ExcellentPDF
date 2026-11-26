# Ph7: 11/32 | F.15: PDF compare

**Tag:** F.15; **Priority:** P2; **Phase:** 7; **Needs:** K.13, V.8; **Code:** `excellent-features::compare`
**Plan:** §27 entry `#### Ph7: 11/32 | F.15`.

## Problem

Reviewing revised contracts, manuscripts, or drawings requires pinpointing both exact word-level text edits and visual layout/pixel differences across hundreds of pages, plus exporting an auditable diff report.

## User story

As a reviewer comparing two PDF versions, I want word-level insertions/deletions, red/blue visual pixel diffs, and annotation change summaries plus an HTML or PDF diff report.

## UI sketch

- Split-pane or overlay compare view with color-coded change markers, changed-page jump list, visual diff heatmap toggle, and "Export Diff Report".

## Mode

Compare / Split view modes.

## Speed and size impact

- Text comparison trims shared prefixes/suffixes before bounded LCS diffing (`diff_words`), keeping 300-page comparison sub-second; pixel diff scans only compared pages when requested.

## Design

- **Text & annotation diff (`diff_words`, `diff_page_texts`, `diff_annotations`)**: Performs prefix/suffix trimming + Myers/LCS word diffing (`WordDiffOp`) per page and detects added/removed/modified annotations (`AnnotationChange`).
- **Visual pixel diff (`diff_pixmaps`)**: Computes pixel mismatch counts and highlights changed regions between rendered `Pixmap` buffers (`VisualDiffResult`).
- **Full compare & export (`compare_pdfs`, `export_compare_html`, `export_compare_pdf`)**: Combines page alignment, text/visual/annotation diffs, and standalone HTML/PDF diff reports.

## Acceptance tests

- [x] `finds_one_word_change_in_300_page_book_and_renders_visual_and_report`: detects a single changed word in a 300-page PDF, verifies rendered visual pixel diff, and validates HTML and PDF report exports.
