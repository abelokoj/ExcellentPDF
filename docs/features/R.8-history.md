# R.8 Back and forward history

**Tag:** R.8 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.5 · **Code:** `excellent-core::view::history`
**Plan:** §27 entry `#### R.8`.

## Problem

After a jump there was no way back to where the reader was.

## User story

As a reader following a cross-reference, I want to go back to where I was, so that I don't lose my place.

## UI sketch

Palette, group **Navigate**: Back (Alt+←), Forward (Alt+→). The mouse's back and forward buttons do the same.

## Mode

Every reading mode.

## Speed and size impact

None measurable: a place is a page anchor and a zoom; at most 256 are kept each way.

## Design

`History` keeps back and forward lists of `Place { anchor, mode, zoom }`. A jump pushes where the reader was and clears
the places ahead; scrolling pushes nothing.

## Acceptance tests

- [x] `three_jumps_and_three_backs_return_to_the_start` (the model; jumps by go to page today).
- [x] `a_new_jump_forgets_the_places_ahead`, `history_is_bounded`.
- [x] Following three links and pressing back three times returns to the start: links (R.7) jump through the same
  path as go to page, which the model test covers; not yet tried by hand on a device.
