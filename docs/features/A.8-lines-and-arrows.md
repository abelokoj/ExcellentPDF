# Ph3: 21/49 | A.8: Lines and arrows

**Tag:** A.8; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::shapes`,
`excellent-canvas::page::canvas::draw`, `excellent-app::annotate`
**Plan:** §27 entry `#### Ph3: 21/49 | A.8`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers point at things on a page and join them up: an arrow to a figure's detail, a line under a column.

## User story

As a reader, I want to drag a line or an arrow onto a page, straight when I hold Shift, and choose how each end
looks, so I can point at what matters and other readers see the same arrow.

## UI sketch

- `annotate.line` (`Shift+L`) and `annotate.arrow` (`Shift+A`), in the palette and the Annotate toolbar: the pointer
  becomes a crosshair and the inspector says "Drag to draw". A drag on a page draws the line as it goes; Shift snaps
  it to steps of 15°. The command again calls it off.
- The inspector, for a line: "Arrow on page 2"; its note; Start and End: None, Open, Closed, Dot, Square, Diamond,
  Slash; Delete line and Done.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

None at startup. While dragging, the canvas draws one polyline a frame.

## Design

- **Model** (`annotate::shapes`, pure): `Kind` (line, arrow), `snap`, `draw`, `set_ends`, `distance` (for the click
  that opens a line), `Drawn` (the canvas's event).
- **Canvas** (`page::canvas::draw`): the `drawing` prop turns the tool on; the drag, its preview with the arrowhead,
  and `pageDraw`. `Host::shift_down` says whether to snap.
- **Colours**: a `Shape` tool in A.3's colours, so the colour dialog sets the shapes' colour.
- **App**: `Placing::Shape`; the drawn line becomes one undoable edit and opens in the inspector, with its endings.

## Acceptance tests

- [x] `an_arrow_keeps_its_ends_through_save_and_reopen` (`/LE[/Circle/ClosedArrow]` in the bytes; read back).
- [x] `random_annotation_edits_survive_save_and_reopen` (lines and paths with random endings).
- [x] `shift_snaps_to_fifteen_degree_steps_at_the_same_length`, `a_drag_makes_a_line_or_an_arrow_and_a_click_makes_nothing`.
- [x] `changing_the_ends_is_one_change_only_when_they_change`, `distance_is_measured_to_the_nearest_point_of_the_line`,
  `the_canvas_s_drawn_shape_parses_into_an_annotation`.
- [ ] Arrow heads look the same in Acrobat: not yet checked by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- Shift snapping reads the key on Windows only; GTK and macOS draw unsnapped until their hosts read it.
- No line width control yet (A.12's inspector has the border width).
- A line can't be moved, nor its ends dragged, until A.13.
