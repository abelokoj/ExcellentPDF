# Ph5: 1/45 | R.16: Reflow (Liquid Mode)

**Tag:** R.16; **Priority:** P1; **Phase:** 5; **Needs:** K.13; **Code:** `ecclesia-features::reflow`
**Plan:** §27 entry `#### Ph5: 1/45 | R.16: Reflow (Liquid Mode)`.

## Problem

Fixed-layout PDFs with multi-column pages or small margins are difficult to read on narrow windows or tablets without zooming and panning horizontally.

## User story

As a reader on a narrow window or tablet, I want to toggle Liquid Reflow mode with custom font, spacing, margins, and theme while keeping selection and highlighting synchronized to the original PDF page.

## UI sketch

- Toolbar / View menu / Command Palette (`read.reflow`, shortcut `Ctrl+Shift+R`) toggles reflow mode.
- Reflow typography popover adjusts font family, font size (`8`–`36` pt), line height (`1.0`–`2.5`), margins, and theme (`Light`, `Sepia`, `Dark`, `Custom`).
- Selecting or highlighting text in reflow maps back to exact PDF page `Quad`s.

## Mode

Read, Annotate, Study (available via View menu and Command Palette).

## Speed and size impact

- Startup: zero cost; extracts structured blocks lazily from K.13 `TextLayout` only when reflow is active.
- Budgets touched: UI-thread stall < 50 ms; pure Rust block classification and HTML emission.
- Installer size: zero new crates.

## Design

- **Model (`ecclesia-features::reflow`):**
  - `ReflowPage::extract` consumes K.13 `TextLayout`, orders blocks with `read::columns::detect_columns`, detects headings, paragraphs, list items, multi-column tables, and captioned figures, and builds `ReflowSpan` mappings back to source `Quad`s.
  - `ReflowPage::reflow_to_page_quads` and `page_point_to_reflow` provide bidirectional coordinate synchronization.
  - `ReflowPage::to_html` emits themed HTML/CSS ready for MuPDF's story engine or NSH `TextView`.

## Acceptance tests

- [x] `reflow::tests::extracts_two_column_reflow_and_maps_selection_quads_bidirectionally`: verifies column-major reading order, heading classification, HTML rendering, and bidirectional `Quad` highlight mapping.
