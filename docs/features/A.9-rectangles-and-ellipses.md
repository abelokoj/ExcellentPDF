# Ph3: 22/49 | A.9: Rectangles and ellipses

**Tag:** A.9; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::shapes`,
`excellent-canvas::page::canvas::draw`, `excellent-app::annotate`
**Plan:** §27 entry `#### Ph3: 22/49 | A.9`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers box a figure, ring a number, or shade a region to set it apart, and expect the box to look the same in
Acrobat.

## User story

As a reader, I want to drag a rectangle or an ellipse onto a page, square or round when I hold Shift, and choose its
line width, line style and fill, so I can mark a region and other readers see the same shape.

## UI sketch

- `annotate.rectangle` (`R`) and `annotate.ellipse` (`Shift+E`), in the palette and the Annotate toolbar: the
  pointer becomes a crosshair and the inspector says "Drag to draw". A drag on a page draws the box from corner to
  corner; Shift makes it a square or a circle. The command again calls it off.
- The inspector, for a shape: "Rectangle on page 2"; its note; Line width: 1 pt, 2 pt, 4 pt; Line style: Solid,
  Dashed, Dotted; for a box, Fill: None, White, Yellow, Blue, Red; Delete shape and Done. Lines and arrows get the
  same width and style rows under their endings.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

None at startup. While dragging, the canvas draws one polyline a frame (an ellipse as 48 sides).

## Design

- **Model** (`annotate::shapes`, pure): `Kind` gains `Rectangle` and `Ellipse`; `constrain` (snap a line, square a
  box), `big_enough`, `outline` (the preview), `Dash`, `FILLS`, `Look`, `set_width`, `set_dash`, `set_fill`, `hits`.
- **Core**: `Annotation::fill` and `Annotation::dash`, through `AnnotData` and the shim, and in the operation log's
  JSON (older logs read as no fill, solid).
- **Shim**: the fill as `/IC` (none writes no `/IC` unless one is there to clear); the dashes rewritten only when
  they change, so another app's border style stays.
- **Canvas**: A.8's drawing tool, with the box's outline as its preview.

## Acceptance tests

- [x] `rectangles_and_ellipses_keep_fill_border_and_dash_through_save_and_reopen` (`/S/D` in the bytes; read back).
- [x] `random_annotation_edits_survive_save_and_reopen` (rectangles and ellipses with fills and dashes).
- [x] `shift_makes_a_box_square_whichever_way_it_is_dragged`,
  `a_drag_makes_a_box_from_corner_to_corner_and_a_thin_one_makes_nothing`, `the_preview_outlines_a_closed_box_and_ellipse`.
- [x] `width_dash_and_fill_are_one_change_each_and_a_line_takes_no_fill`,
  `a_click_hits_a_box_s_border_and_a_filled_box_s_inside`.
- [ ] The shapes look the same in Acrobat: not yet checked by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- Shift makes squares and circles on Windows only; GTK and macOS draw them free until their hosts read the key.
- A shape can't be moved or resized until A.13.
