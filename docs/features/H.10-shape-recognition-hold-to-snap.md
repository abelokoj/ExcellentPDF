# Ph3: 46/49 | H.10: Shape recognition (hold to snap)

**Tag:** H.10; **Priority:** P1; **Phase:** 3; **Needs:** H.1; **Code:** `excellent-ink::shapes` (the recognizer),
`excellent-canvas::page` (the hold, and the stroke replaced)
**Plan:** §8.4 and §27 entry `#### Ph3: 46/49 | H.10`.

## Problem

Lines, boxes and circles drawn by hand come out wobbly; drawing them true meant switching to the shape tools (A.8).

## User story

As someone sketching a diagram in my notes, I want to draw a box or an arrow by hand, rest the pen a moment, and have
it come out straight, without leaving the pen.

## UI sketch

Draw a shape, keep the pen on the screen without moving for half a second, lift: the stroke becomes the shape, in the
same pen. Lift straight away and the stroke stays as drawn.

## Mode

Write.

## Speed and size impact

One recognition per held stroke on pen-up: a simplification and two fits over its points, well under a millisecond.

## Design

- **Open strokes:** a line when every point lies close to the line between its ends; an arrow when the stroke runs out
  as a straight shaft and then turns back about its tip (the head drawn in the same stroke). The arrow comes out with
  two 30-degree barbs a quarter of the shaft long.
- **Closed strokes** (ends within 18 % of the stroke's length): the corners by Douglas-Peucker (6 % of the stroke's
  size), dropping places where the outline barely turns, against an ellipse from the stroke's second moments. The
  polygon wins only when it fits clearly better. Four corners near right angles come out a true rectangle; an ellipse
  with radii within 12 % comes out a circle.
- **Refused:** a stroke whose points stray more than 6 % of its size from the best fit, a polygon of more than eight
  corners, and a stroke much longer than the shape's own outline (a scribble, shading).
- **The hold:** the canvas notes when the pen last moved more than 3 pixels; held 500 ms before it lifts, the smoothed
  stroke goes through the recognizer, and the shape's path, at the stroke's mean width, is what is saved and shown.

## Acceptance tests

- [x] Recognition accuracy is at least 95 % on a set of 500 shapes:
  `recognizes_at_least_95_percent_of_500_hand_drawn_shapes` (lines, arrows, triangles, rectangles, pentagons,
  circles and ellipses, drawn with a wobble and a jitter, started anywhere and closed short or past the start: 99.4 %).
- [x] `a_scribble_stays_a_scribble`, `shapes_draw_true`.

## Open

- The 500 shapes are synthetic, made to look hand drawn; redo the set from recorded strokes (DD24).
- The shape shows when the pen lifts; a preview while the pen is still held is not built.
- Shapes come out as ink in the pen's style, not as the shape tools' annotations (A.8).
