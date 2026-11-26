# Ph5: 3/45 | R.24: Two-column auto-detect

**Tag:** R.24; **Priority:** P2; **Phase:** 5; **Needs:** K.13; **Code:** `excellent-features::read::columns`
**Plan:** §27 entry `#### Ph5: 3/45 | R.24: Two-column auto-detect`.

## Problem

Reading academic two-column PDFs at comfortable zoom requires manually scrolling back to the top of the page after finishing the left column.

## User story

As a researcher reading two-column papers, I want column mode to detect columns automatically and jump cleanly from the bottom of column 1 to the top of column 2.

## UI sketch

- View menu / Command Palette (`read.column_mode`) toggles column-guided scrolling.
- Status bar shows detected layout (`Single column` / `Two columns`).

## Mode

Read, Study.

## Speed and size impact

- Startup: zero cost; inspects K.13 `TextLayout` blocks on demand.
- Budgets touched: < 0.2 ms per page for gutter search.
- Installer size: zero new crates.

## Design

- **Model (`excellent-features::read::columns`):**
  - `detect_columns` scans the central `[35%..65%]` page band for a vertical gutter, separates full-width headers/titles (`FullWidthTop`) and footers (`FullWidthBottom`) from left/right columns, and emits `reading_order` block indices.
  - `ColumnScrollNavigator` advances vertically within column 1 and emits `ColumnScrollTarget::JumpToNextColumn` to the top of column 2 before advancing to the next page.

## Acceptance tests

- [x] `read::columns::tests::detects_two_columns_with_full_width_title_and_orders_left_then_right`: verifies two-column detection, reading order, and column-jump navigation.
- [x] `read::columns::tests::single_column_page_stays_single`: ensures single-column pages are not falsely split.
