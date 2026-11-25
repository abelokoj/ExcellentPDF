# Ph3: 45/49 | H.9: Save ink as standard PDF

**Tag:** H.9; **Priority:** P0; **Phase:** 3; **Needs:** H.1, K.14; **Code:** `excellent-ink::outline` (a stroke's
outline), `excellent-core::annot` (`Annotation::widths`, the outlines written), `excellent-mupdf` / the shim
(`/ExcellentWidths`, the appearance), `excellent-features` (the ink event's widths, the eraser keeping them)
**Plan:** §8.3 and §27 entry `#### Ph3: 45/49 | H.9`.

## Problem

A PDF ink annotation has one width. Until now a pressure pen's stroke, thin where the pen touched lightly and broad
where it pressed, was saved and shown as a line of its mean width.

## User story

As a note-taker, I want my handwriting to look in every PDF reader the way it looked as I wrote it.

## UI sketch

Nothing to see but the ink itself, which keeps its shape once the pen lifts, after saving and in other readers.

## Mode

Write.

## Speed and size impact

Per stroke: one outline (two points per sample and 14 for the round ends) computed when the annotation is written,
and a fill in its appearance stream; a few kilobytes for a long stroke.

## Design

- **InkList:** the smoothed points, as before (§8.3), so readers that redraw ink from the list still get the stroke.
- **Constant-width pens** (ballpoint, marker, highlighter; widths within 2 %): MuPDF's own appearance
  (`pdf_update_annot`), as the plan says; the highlighter keeps patch `0094`'s Multiply.
- **Pressure pens:** each point's width goes into the private `/ExcellentWidths` key, and the appearance is the
  strokes' outlines (`excellent-ink::outline`: each side offset by half the width at each point, round ends), filled
  in the annotation's colour and opacity (multiplied for a highlighter) through a display list and
  `pdf_set_annot_appearance_from_display_list`. The outlines are drawn in the page's unrotated PDF space and `/Rect`
  becomes their bounds, so the form's bounding box maps onto it in every reader.
- **Kept through edits:** the widths live on `Annotation::widths` (one list per stroke) and in the operation log, so a
  move, a resize, a turn, undo, redo and recovery all write the outline again. The area eraser keeps the widths of the
  strokes it leaves; the pieces of a cut one take the annotation's one width.

## Acceptance tests

- [ ] Ink looks identical in Acrobat, PDF-XChange and pdf.js (K.26): not run here; the maintainer's to check.
- [x] `a_pressure_stroke_is_saved_and_drawn_as_written`: a stroke from 1 to 15 points is drawn thin at its start and
  broad at its end by MuPDF, again from the saved and reopened file, and its widths come back.
- [x] The outline: `a_tapering_stroke_is_as_wide_as_each_point`, `the_ends_bulge_outwards`,
  `a_dot_is_round_and_nothing_is_nothing`, `even_strokes_are_told_apart_from_pressure_strokes`.

## Open

- Raw samples (pressure, tilt, time) in a compressed private key, for replay (H.22) and audio sync (L.1): not built;
  the canvas keeps them per stroke but doesn't send them yet.
- A pencil's grain shows in the golden images only (DD25); its appearance is a plain fill.
- The pieces of an area-erased pressure stroke lose their widths.
- `bindgen` needed `prettyplease` 0.2: the lockfile had paired 0.3 (built on `syn` 3) with `bindgen`'s `syn` 2.
