# Ph4: 39/44 | G.4: Magnifier

**Tag:** G.4; **Priority:** P0; **Phase:** 4; **Needs:** G.1, K.8; **Code:** `ecclesia-features::present::pointers`
**Plan:** §27 entry `#### Ph4: 39/44 | G.4`.

## Problem

Small diagrams or fine print on slides are unreadable from the back of a lecture hall.

## User story

As a presenter, I want a magnifier lens that enlarges the region under my pointer directly from higher-zoom tiles in the same frame.

## UI sketch

- Present toolbar: Magnifier lens toggle, zoom factor (`1.25x..=8.0x`, default `2.5x`).

## Mode

Present mode.

## Speed and size impact

- Samples existing higher-zoom tiles; zero full-page re-render.

## Design

- `MagnifierLens::view_for_page` computes `MagnifierView { lens_rect, source_rect, zoom_factor }` clamped to page bounds in the same frame.

## Acceptance tests

- [x] `spotlight_and_magnifier_track_pointer_in_same_frame`: verifies exact lens/source ratio and same-frame tracking.
