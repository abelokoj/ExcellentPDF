# Ph4: 6/44 | H.13: Handwriting straightening

**Tag:** H.13; **Priority:** P1; **Phase:** 4; **Needs:** H.6; **Code:** `ecclesia-ink::straighten`
**Plan:** §8.4 and §27 entry `#### Ph4: 6/44 | H.13`.

## Problem

Handwritten notes written on blank pages or in margins often drift uphill or downhill across a line. Fixing a sloped paragraph meant selecting and rotating every line by hand.

## User story

As someone taking freehand notes on blank paper, I want to lasso a sloped paragraph and straighten it in one step so that every line sits level while my handwriting keeps its exact shape.

## UI sketch

Available in the lasso selection toolbar, the Write menu, the command palette (`ink.straighten`, "Straighten handwriting"), and keyboard shortcut `Ctrl+Shift+L` (when a lasso selection is active). Runs in one undoable operation; shows a subtle status toast with the number of lines leveled. Accessible name: "Straighten selected handwriting".

## Mode

Write (active when ink strokes are selected with the lasso).

## Speed and size impact

- Startup: nothing runs at startup; pure math in `ecclesia-ink` invoked only on command.
- Budgets touched: UI-thread stall < 50 ms (linear/quadratic over selected strokes, < 1 ms for a typical paragraph).
- Installer size: negligible (~4 KB compiled Rust, zero dependencies).

## Design

- **Model (`ecclesia-ink::straighten`):**
  1. Estimates the paragraph's dominant slope from nearest right-hand neighbor strokes (ignoring small diacritics such as `i`-dots).
  2. Projects stroke centroids perpendicular to the dominant slope to group strokes into lines cleanly even when lines are steeply tilted; attaches diacritics to their nearest line.
  3. Fits each line's baseline on stroke bottoms via Huber iteratively reweighted least squares (IRLS) so descenders (`g`, `p`, `y`) do not pull the baseline downward.
  4. Applies a single rigid Euclidean transform (`rigid_transform`: pure rotation by `-angle` plus optional `LineSpacing::EvenSpacing` vertical shift) to every stroke on a line, preserving all segment lengths, turning angles, and per-point widths (`straighten_ink`).
- **Command id (for app wiring):** `ink.straighten` (shortcut `Ctrl+Shift+L` with selection).
- **Storage:** commits updated `Shape::Ink` coordinates in one K.14 `Operation`.

## Acceptance tests

- [x] `sloped_paragraph_becomes_level_without_changing_letter_shapes`: a 3-line paragraph of multi-stroke letters (`H`, `i` with dot, `o`, `g` with descender) sloped at 14° levels to < 0.6° residual slope while preserving every segment length and diacritic offset within `1e-4` pt.
- [x] `negative_slope_and_even_spacing_level_cleanly`: handles negative slopes (-16°) and `LineSpacing::EvenSpacing`.
- [x] `ink_points_keep_their_per_point_widths_and_empty_input_is_safe`: preserves per-point widths and handles empty inputs.
