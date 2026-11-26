# Ph4: 8/44 | H.15: Bulk restyle

**Tag:** H.15; **Priority:** P1; **Phase:** 4; **Needs:** H.6; **Code:** `excellent-features::ink::restyle`
**Plan:** §8.4 and §27 entry `#### Ph4: 8/44 | H.15`.

## Problem

Changing the color, thickness, or pen type of dozens of strokes across a page or selection requires editing them one by one or redrawing them.

## User story

As a reader organizing handwritten notes, I want to change the color, width, opacity, or pen type of all selected strokes or all strokes matching a filter (such as "all red ink on this page") and undo the entire change in one step if I change my mind.

## UI sketch

Reached from the lasso selection panel ("Restyle..."), the Write menu, the command palette (`ink.bulk_restyle`, "Bulk restyle ink"), and shortcut `Ctrl+Shift+R` (in Write mode). Offers scope ("Selection" vs filter by page, color, width range, highlighter/pen, layer, author) and target color, opacity, width (set or scale), and pen type. Accessible controls with keyboard focus order.

## Mode

Write.

## Speed and size impact

- Startup: nothing runs at startup; executes only when the user invokes bulk restyle.
- Budgets touched: single-pass filter and `Operation` build (< 1 ms for 1,000 strokes).
- Installer size: negligible (~4 KB compiled Rust, zero new dependencies).

## Design

- **Model (`excellent-features::ink::restyle`):**
  - `StrokeFilter` selects ink annotations by IDs (selection), page, RGB `ColorMatch` with tolerance, width range, `InkCategory` (pen vs highlighter), layer, or author.
  - `RestyleSpec` updates color, opacity, width (`WidthChange::Set` / `WidthChange::Scale` with proportional per-point pressure-width scaling), and `PenKind` (adjusting `multiply`, pen opacity, and flat vs variable widths).
  - `restyle_operation` packs every `Change::UpdateAnnotation` into a **single** K.14 `Operation`, skipping locked annotations (A.12) and locked layers (H.14).
- **Command id (for app wiring):** `ink.bulk_restyle` (`Ctrl+Shift+R` in Write mode).
- **Storage:** standard PDF annotations updated atomically in one undoable K.14 operation.

## Acceptance tests

- [x] `one_undo_reverts_the_whole_restyle`: creates a real PDF via MuPDF with red and blue ink strokes, applies a single `restyle_operation` matching "all red ink on page 0" (recoloring to green, widening to 5 pt, switching to Marker), verifies only the 3 red strokes changed, then executes **one** `Document::undo()` and verifies all 3 strokes revert completely.
- [x] `pen_type_and_proportional_widths_and_locks_behave_correctly`: verifies proportional pressure-width scaling, pen type conversion to Highlighter, and skipping strokes on locked layers.
