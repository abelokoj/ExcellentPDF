# Ph3: 41/49 | H.5: Erasers

**Tag:** H.5; **Priority:** P0; **Phase:** 3; **Needs:** H.1; **Code:** `ecclesia-ink::erase` (the geometry),
`ecclesia-features::annotate::erase` (a pass to annotation changes), `ecclesia-canvas::page` (`eraser` prop, the
eraser end, the `erase` event), `ecclesia-app` (`ink.eraser_*` commands, the Write toolbar)
**Plan:** §8.4 and §27 entry `#### Ph3: 41/49 | H.5`.

## Problem

Ink could be drawn but not taken off again, except by selecting a whole annotation and deleting it.

## User story

As a note-taker, I want to flip my pen over and rub out what I wrote, a whole stroke or only the part under the
eraser, and to take off highlighting without touching my handwriting.

## UI sketch

The Write toolbar, after the pens: Eraser, Area eraser, Highlight eraser (keys 8, 9 and 0 on the pages; "Stroke
eraser", "Area eraser" and "Highlighter eraser" in the palette under Ink). The chosen one shows pressed; choosing a
pen puts the eraser away. The stylus's eraser end (or the pen turned over) erases with the last eraser chosen, the
stroke eraser at first, whatever is chosen on the toolbar. While erasing, a light grey swath shows where the eraser
went; on pen-up the ink it crossed goes.

## Mode

Write (the toolbar); the commands and the eraser end work in any mode.

## Speed and size impact

Nothing at startup. A pass is computed once, on pen-up, over the page's shown annotations: each stroke segment is
tested against each eraser capsule after a bounding-box check, so only nearby pairs cost the exact search.

## Design

- **Geometry (`ecclesia-ink::erase`):** the eraser's path with its radius is a chain of capsules. Its reach is the
  radius plus half the stroke's width, so it takes what it covers of the line, not only its centre. A capsule is
  convex, so a stroke segment meets it in one interval, found by a ternary search for the nearest point and
  bisection for each edge. The intervals are merged and cut out; pieces shorter than 0.5 pt are dropped.
  - Stroke eraser: a stroke it touches is removed whole.
  - Area eraser: a stroke comes out as the pieces left, split in two where the eraser crossed its middle.
  - Highlighter eraser: as the stroke eraser, on highlighter ink (H.3) only.
- **Changes (`ecclesia-features::annotate::erase`):** for each ink annotation on the page, its strokes after the pass.
  One with strokes left is written again (`UpdateAnnotation`, same id), one with none is deleted. All go in one
  operation, so one undo restores everything. Locked annotations (A.12) are left alone, and so is ink on hidden layers
  (A.22).
- **Canvas:** the `eraser` prop (a name, or none) and the eraser end start a pass instead of a stroke; the radius is
  10 pixels on screen in page points at the zoom it starts at. Pen-up sends `erase` (`{page, eraser, radius,
  points}`).

## Acceptance tests

- [x] Area-erasing through a stroke leaves two correct strokes, both undoable in one step:
  `area_erasing_through_a_stroke_leaves_two_strokes_undone_in_one_step` (a real PDF through the operation log,
  saved and reopened, then undone), and the model's `area_erasing_through_a_stroke_leaves_two_correct_strokes`.
- [x] The erasers: `a_stroke_eraser_removes_what_it_touches_and_nothing_else`,
  `the_highlighter_eraser_leaves_pen_ink`, `erasing_an_end_shortens_the_stroke_and_erasing_all_removes_it`,
  `crumbs_go_with_the_cut`, `a_segment_meets_a_capsule_in_one_exact_interval`.
- [x] The changes: `area_erasing_through_a_stroke_leaves_two_strokes_in_one_change`,
  `a_stroke_eraser_deletes_and_leaves_other_annotations`, `one_stroke_of_two_goes_and_the_annotation_stays`,
  `locked_and_highlighter_rules_hold`, `a_broken_pass_is_refused`.

## Open

- Ink disappears on pen-up, not live under the eraser.
- An area eraser's pieces stay in the one annotation they came from (an ink list of several strokes), so they select
  and move together until the lasso (H.6) can split them.
- The eraser's size is fixed until H.7's presets.
