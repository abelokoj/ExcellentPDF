# Ph4: 42/44 | G.7: Curtain reveal

**Tag:** G.7; **Priority:** P0; **Phase:** 4; **Needs:** G.1; **Code:** `ecclesia-features::present::curtain`
**Plan:** §27 entry `#### Ph4: 42/44 | G.7`.

## Problem

Teachers walking through worked examples or quiz answers want to conceal the bottom (or side) of a slide and reveal it incrementally line by line.

## User story

As a teacher, I want a movable curtain overlay that I can step down with keyboard keys or drag smoothly with my pen.

## UI sketch

- Present toolbar: Curtain toggle, direction selector (Top-to-Bottom, Bottom-to-Top, Left-to-Right, Right-to-Left), draggable handle bar.

## Mode

Present mode.

## Speed and size impact

- Pure rectangle overlay; zero tile re-renders.

## Design

- `CurtainReveal` with `CurtainDirection`, `step_reveal`, `step_hide`, `cover_all`, `reveal_all`, `drag_to(point, page_bounds)`, and `covered_rect(page_bounds)`.

## Acceptance tests

- [x] `keyboard_and_pen_drag_both_move_curtain_accurately`: verifies full coverage, keyboard step increments, pen drag positioning, horizontal reveal direction, and full reveal.
