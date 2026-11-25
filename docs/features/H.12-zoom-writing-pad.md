# Ph4: 5/44 | H.12: Zoom writing pad

**Tag:** H.12; **Priority:** P1; **Phase:** 4; **Needs:** H.1; **Code:** `excellent-ink::zoompad` (the model; the plan
names `excellent-features/ink/zoompad.rs`, but the pad is pen geometry with no app state, so it sits with the other
ink models), `excellent-canvas::page` (the pad and its box), `excellent-app` (`ink.zoom_pad`)
**Plan:** §8.4 and §27 entry `#### Ph4: 5/44 | H.12`.

## Problem

Writing small and neat with a pen on a page at normal zoom is hard; zooming in to write means scrolling for every
line.

## User story

As a note-taker on a tablet, I want to write comfortably large and have it land small and neat on the page, line
after line, without moving anything by hand.

## UI sketch

Zoom pad on the Write toolbar (Alt+Z on the pages, the palette). The bottom third of the view becomes a pad with a
writing line; a box on the page shows where the writing lands. Write in the pad: each stroke lands in the box at a
third of the size, and the pad shows what is in the box, magnified. When the writing nears the pad's right edge the
box steps half its width along the line, and at the end of the line it goes to the start of the next.

## Mode

Write.

## Speed and size impact

Off: nothing. On: the pad and the strokes in its box drawn each frame (a few dozen polylines).

## Design

- **The box:** the pad's size over the magnification (3) and the view's scale, starting at the top of the writing area
  (10 % margins left and right, 8 % at the foot) of the page in view.
- **Strokes:** a pen stroke begun in the pad is the pen's own, its samples mapped into the box, its widths divided by
  the magnification when it lands, so the writing on the page looks as if written at that size.
- **Stepping on:** after a stroke, writing past three quarters of the pad steps the box half its width along the line
  (the writing then sits mid-pad); past the line's end, it goes to the next line, a quarter of its height below. At
  the foot of the page the box stays on the last line.

## Acceptance tests

- [x] Writing a paragraph in the pad needs no manual box moves: `writing_a_paragraph_needs_no_manual_box_moves` (25
  words written as a hand does, each continuing where the pad shows the writing ending: they run onto new lines by
  themselves, stay inside the margins and never overlap).
- [x] `the_pad_shows_the_page_magnified`, `the_box_stays_on_the_page_at_its_foot`.

## Open

- The pad shows only what was written in this box this time: ink already on the page under the box isn't shown in it.
- Moving the box by hand, and a "new line" key, are not built; nothing needs them yet.
- The next page when the box reaches the foot is the reader's to turn.
