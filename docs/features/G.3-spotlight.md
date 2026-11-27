# Ph4: 39/44 | G.3: Spotlight

**Tag:** G.3; **Priority:** P0; **Phase:** 4; **Needs:** G.1, K.8; **Code:** `ecclesia-features::present::pointers`
**Plan:** §27 entry `#### Ph4: 39/44 | G.3`.

## Problem

Dense slides overwhelm audiences when the presenter is discussing a single equation, chart, or row.

## User story

As a presenter, I want a spotlight that dims everything outside a circle or rectangle around my pointer in the same frame.

## UI sketch

- Present toolbar: Spotlight button, circle/rectangle toggle, dim opacity slider.

## Mode

Present mode.

## Speed and size impact

- Canvas overlay only; zero PDF tile re-renders.

## Design

- `Spotlight`, `SpotlightShape::{Circle, Rectangle}`, `SpotlightOverlay`.

## Acceptance tests

- [x] `spotlight_and_magnifier_track_pointer_in_same_frame`: verifies immediate same-frame spotlight position tracking and overlay generation.
