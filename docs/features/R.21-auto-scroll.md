# R.21 Auto-scroll

**Tag:** R.21 · **Priority:** P1 · **Phase:** 1 · **Needs:** K.7 · **Code:** `excellent-features::read::autoscroll`,
`excellent-canvas-win::canvas::reading`, `excellent-app::reading`
**Plan:** §27 entry `#### R.21`.

## Problem

Reading a long document hands-free (at a stand, while taking notes on paper) means scrolling every few seconds.

## User story

As a reader, I want the pages to scroll on their own at a speed I set, so that I can keep my hands free.

## UI sketch

Palette, group **Reading**: Auto-scroll (`Ctrl+Shift+H`), Pause or resume auto-scroll (`Space`; without auto-scroll,
a screen down), Auto-scroll faster (`+`), Auto-scroll slower (`−`). The plain keys work while the pages have the
keyboard; `+` and `−` act only while auto-scroll runs (with `Ctrl` they zoom). Middle-click starts scrolling toward
the pointer, faster the farther it is; another click stops it. It pauses at the end of the document. With the ruler
or focus mode on, the band holds its place and the lines pass through it.

## Mode

Every reading mode. Off by default.

## Speed and size impact

- Startup: nothing.
- While it runs, each frame asks for the next, so it moves at the display's refresh rate (the swap chain's wait paces
  it); paused or off, no frame is asked for and nothing runs.

## Design

Model (`read::autoscroll`): `SPEEDS` (Enhanced's 17 steps, 8 to 320 px/s at 96 dpi), `AutoScroll` and `Pan` turn the
time since the last frame into whole pixels and carry the fraction, with a frame counted at most 0.1 s after a stall.
The canvas reports a changed speed (`autoScrollSpeed`), which the app keeps in `auto_scroll_speed`.

## Acceptance tests

- [x] `frames_of_any_rate_cover_the_same_distance` (60, 120 and 144 Hz), `it_moves_by_whole_pixels`.
- [x] `paused_it_moves_nothing_and_a_stall_does_not_jump`, `speeds_step_through_the_table_and_stop_at_its_ends`,
  `middle_click_scrolls_toward_the_pointer_faster_the_farther_it_is`.
- [ ] Smooth at 60 and 120 Hz on a device, and no CPU while paused: not measured (needs K.24's frame timing).
