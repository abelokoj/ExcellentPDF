# Ph3: 20/49 | A.7: Callouts with leader lines

**Tag:** A.7; **Priority:** P0; **Phase:** 3; **Needs:** A.6; **Code:** `ecclesia-features::annotate::freetext`,
`ecclesia-core::annot` (`Shape::Callout`, `LineEnd`), `ecclesia-mupdf-sys/csrc/shim.c`, `ecclesia-app::annotate`
**Plan:** §27 entry `#### Ph3: 20/49 | A.7`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A comment about one part of a figure must point at it: a box of text in the margin, with a line to the part.

## User story

As a reader, I want to click on something on a page and write about it in a box beside it, with an arrow pointing
at what I clicked, so the comment stays clear and the arrow stays put when I move the box.

## UI sketch

- `annotate.callout` (`O`), in the palette and the Annotate toolbar: the inspector says "Click what the callout
  points at"; the next click is the tip, and the box goes above and to the right of it (left or below when the page
  has no room). The command again calls it off.
- The inspector: the text box's controls (A.6), its border, and Arrow: Open, Closed, Dot, Square, None.

## Mode

All modes; the toolbar button is in Annotate mode's toolbar.

## Speed and size impact

None at startup. MuPDF draws the line with the box's appearance.

## Design

- **Model** (`annotate::freetext`): `Kind::Callout`, `place` (tip at the click, box offset by 36 points, kept on the
  page), `set_end`, `kind_of`; fitting the box keeps the tip.
- **Core**: `Shape::Callout(PageRect, PagePoint)`, a free text whose intent is `FreeTextCallout`; `Annotation::ends`
  (`[LineEnd; 2]`), the tip's ending first; both through the log's JSON.
- **Shim**: props read `/CL` and `/LE`; writing sets the callout point and style for a callout, the line ending
  styles for lines, polylines and polygons, and deletes the `/CL` MuPDF's rect setter adds to other text boxes.

## Acceptance tests

- [x] `moving_a_callout_s_box_keeps_its_tip_in_place` (`/IT/FreeTextCallout` in the bytes; tip, box and arrow read
  back after a move, a save and a reopen).
- [x] `random_annotation_edits_survive_save_and_reopen` (now with callouts and line endings).
- [x] `a_callout_points_at_the_click_from_a_box_that_has_room`.
- [ ] Handles for the text box and the arrow tip: wait for A.13.

## Open

- Not tried by hand; not opened in Acrobat.
- No drag handles yet (A.13): a callout is moved by editing its box through the model only.
- The line is written from its tip alone, so a three-point line with a knee from another app loses its knee when the
  box is edited here.
