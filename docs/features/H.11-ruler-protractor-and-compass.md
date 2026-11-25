# Ph4: 4/44 | H.11: Ruler, protractor and compass

**Tag:** H.11; **Priority:** P1; **Phase:** 4; **Needs:** H.1; **Code:** `excellent-ink::guides` (the model),
`excellent-canvas::page` (the guide drawn, moved and turned; strokes fitted to it), `excellent-app::annotate::guides`
(commands, the reading)
**Plan:** §8.4 and §27 entry `#### Ph4: 4/44 | H.11`.

## Problem

Straight lines, angles and circles drawn by hand come out wavering; geometry, diagrams and underlining need them true.

## User story

As a student drawing a geometry figure, I want to lay a ruler, a protractor or a compass on the page and draw along
it, getting a true line or arc and its length or angle.

## UI sketch

Ruler, Protractor and Compass on the Write toolbar (Alt+R, Alt+P and Alt+C on the pages, the palette), each on or off.
The guide lies in the middle of the page in view: a translucent ruler with ticks every 10 points, a half disc with
ticks every 10 degrees, or a circle with its radius. Drag its body to move it; drag its round grip to turn the ruler
or the protractor, or to set the compass's radius. Draw along it with the pen: the stroke comes out true, and the
title bar shows what it measures ("62.4 mm at 30°", "Radius 18.0 mm, 90°").

## Mode

Write.

## Speed and size impact

Nothing while no guide is on. With one: a few dozen short lines a frame, and one fit per stroke on pen-up.

## Design

- **Ruler:** a stroke whose every point lies within 6 points of the edge comes out as one straight line on the edge
  itself, from the first point's place along it to the last's, kept within the ruler's length.
- **Protractor:** a stroke that starts at the centre and runs straight comes out at the nearest whole degree, as long
  as drawn.
- **Compass:** a stroke whose every point lies within 6 points of the circle comes out as the exact arc, as far round
  as it went (summed turn by turn, so a full circle is a full circle).
- **Drawn true:** the fitted path, at the stroke's mean width, is what is saved; strokes that don't lie along the
  guide are left as drawn.
- Moving and turning are mouse drags; two-finger moves wait for touch gestures on the canvas.

## Acceptance tests

- [x] A stroke along the ruler is exactly straight: `a_stroke_along_the_ruler_is_exactly_straight` (a wobbling hand at
  30 degrees comes out on the edge, 200 points long at 30 degrees).
- [x] `a_stroke_off_the_ruler_is_left_alone`, `a_stroke_round_the_compass_is_an_exact_arc`,
  `a_stroke_from_the_protractor_lands_on_a_whole_degree`.

## Open

- Moving and turning a guide with two fingers (it needs the canvas's touch gestures).
- One guide at a time, laid on the page in view; it isn't kept when the app closes.
