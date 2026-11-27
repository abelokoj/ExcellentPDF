# Ph5: 7/45 | B.8: Generate an outline

**Tag:** B.8; **Priority:** P1; **Phase:** 5; **Needs:** B.1, K.13; **Code:** `ecclesia-features::outline_extra::generate`
**Plan:** §27 entry `#### Ph5: 7/45 | B.8: Generate an outline`.

## Problem

Scanned books, lecture handouts, and combined PDFs often lack a bookmark outline, forcing readers to scroll blindly through hundreds of pages.

## User story

As a reader of an untagged PDF book, I want EcclesiaPDF to infer a structured outline from heading font sizes, numbering rules, or structure tags and show an editable preview tree before saving.

## UI sketch

- Outline sidebar > Generate Outline (`outline.generate`).
- Preview dialog renders `OutlinePreviewTree` with checkboxes (`NodeDecision::Accept`/`Discard`), inline rename, and Apply / Cancel.

## Mode

All modes (editing enabled for PDFs; preview available everywhere).

## Speed and size impact

- Startup: zero cost; scans K.13 `TextLayout`s on demand.
- Installer size: zero new crates.

## Design

- **Model (`ecclesia-features::outline_extra::generate`):**
  - `detect_numbering_level`: recognizes decimal (`1.`, `1.2.`), Roman (`I.`, `II.A.`), alpha (`A.`), and named prefixes (`Chapter 1`, `Section 2.1`, `Appendix A`).
  - `generate_from_layouts`: estimates median body font size, clusters heading tiers, and builds a nested `OutlinePreviewTree`.
  - `from_structure_tags`: extracts tagged PDF `H1`–`H6`/`Part`/`Sect` trees.
  - `for_combined_files`: generates top-level bookmarks per combined document (O.2).

## Acceptance tests

- [x] `outline_extra::generate::tests::generates_nested_outline_from_headings_and_numbering_with_preview_editing`: verifies font-scale + numbering hierarchy and preview rename/discard.
- [x] `outline_extra::generate::tests::generates_from_structure_tags_and_combined_files`: verifies structure-tag and combined-file outline creation.
