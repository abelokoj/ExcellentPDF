# Ph3: 42/49 | H.6: Lasso

**Tag:** H.6; **Priority:** P0; **Phase:** 3; **Needs:** H.5, A.13; **Code:** `ecclesia-ink::lasso` (the loop and
turning), `ecclesia-features::annotate::lasso` (what a loop takes; resizing, turning, widths and colours of a
selection), `ecclesia-canvas::page` (`lasso` prop, `annotLasso`), `ecclesia-app` (`ink.lasso`, `select.*`, the
inspector's selection panel)
**Plan:** §8.4 and §27 entry `#### Ph3: 42/49 | H.6`.

## Problem

A.13 selects annotations by clicking or by a rectangle, which can't pick out handwriting that overlaps other
writing, and works on them one at a time: resizing, turning or restyling a whole drawing was not possible.

## User story

As a note-taker, I want to draw a loop around a word or a sketch I wrote, then move it, scale it, turn it, make it
thicker or another colour, or duplicate it, all at once.

## UI sketch

Lasso is on the Write toolbar after the erasers (V on the pages, "Lasso" in the palette). With it on, a drag on a
page with the pen or the mouse draws a blue loop; on release what it encloses is selected (Ctrl adds to the
selection). The rectangle lasso is A.13's marquee, a drag with the lasso off. Several selected show one box around
them with corner grips: a corner scales them all, the inside moves them all, and every outline follows a move. The
inspector shows "N selected" with the palette's colours and Rotate left, Rotate right, Thinner, Thicker and Duplicate.
The same commands are in the palette with Ctrl+Shift+[ and ] to turn, Alt+[ and ] for width, Ctrl+Shift+D to
duplicate.

## Mode

Write (the toolbar); the selection commands work in any mode.

## Speed and size impact

Nothing at startup. A loop is tested on release only: an even-odd test per point. A move drag offsets outlines; the
edit is computed once, on release. Measured in a release build: 0.9 ms to select 1,000 strokes of 40 points and 1.6
ms to compute their move.

## Design

- **Loop (`ecclesia-ink::lasso`):** Enhanced's `LassoContains` (even-odd) and `RotateLassoPoint`, ported. A loop needs
  three points.
- **What it takes:** ink when every point is inside; anything else when its four corners are (Enhanced's rule, so
  separate strokes of a word are taken and a box half in is not).
- **Group box:** with two or more selected, the app's `selected` prop starts with an item of id `*selection*`, the
  box around them with four corner grips; the items' own grips give way. A corner drag on it scales every movable one
  from the box (`group_resized`); a drag inside moves them all, as A.13's moves do.
- **Turning:** a quarter turn about the box's centre, for ink, lines, polygons and polylines; nothing turns unless
  all can (a PDF box stays upright), as in Enhanced.
- **Widths:** × 0.8 or × 1.25 within 0.1 to 72 pt (Enhanced's steps and limits), for kinds that have a width.
- **Duplicate:** A.13's clipboard paste (new ids, 8 pt down and right) without touching the clipboard; the copies
  become the selection.
- Each is one step of undo; locked annotations stay as they are.

## Acceptance tests

- [x] A lasso move of 1,000 strokes stays at 60 fps: `a_lasso_move_of_a_thousand_strokes_fits_a_frame` (selection
  and the move each within one frame; asserted in release builds).
- [x] The loop: `a_loop_holds_what_is_inside_it`, `a_concave_loop_leaves_its_notch_out`, `two_points_enclose_nothing`,
  `turning_goes_clockwise_on_the_page_and_back`.
- [x] The selection: `a_loop_takes_ink_wholly_inside_and_boxes_by_their_corners`, `a_loop_needs_three_points`,
  `the_selection_scales_from_its_own_box`, `turning_goes_about_the_centre_and_needs_every_one_to_turn`,
  `widths_step_within_their_limits_and_colours_change`, `locked_ones_stay`.

## Open

- Enhanced has no lasso tests to port; these are new.
- Turning is by quarter turns from commands; a free-turning handle on the box is not built.
- Copy as image (with O.23's region rendering) and convert to text (H.24) are not built.
- The frame rate of a drag on a real page is the maintainer's to measure; the test covers the app's work around it.
