# Ph3: 44/49 | H.8: Palm rejection and "finger draws"

**Tag:** H.8; **Priority:** P0; **Phase:** 3; **Needs:** H.1; **Code:** `excellent-canvas::page::gestures` (the
model), `excellent-canvas::page` (the canvas's touch entry points, `fingerDraws` prop), `excellent-canvas-win`
(`WM_POINTER*` touch and pen hover), `excellent-app` (the setting)
**Plan:** §7.3 and §27 entry `#### Ph3: 44/49 | H.8`.

## Problem

Writing with a pen, the hand rests on the screen. On Windows a touch the canvas didn't take became a mouse press, so
a resting palm could press, select or drag on the page.

## User story

As a note-taker on a Surface, I want to rest my hand on the screen while I write and get no stray marks or jumps; and,
when I choose, to draw with a finger instead of scrolling.

## UI sketch

Settings, Modes: "Finger draws" (off by default), under "Pen switches to Write". Nothing else shows: a resting hand
simply does nothing.

## Mode

Every mode.

## Speed and size impact

None measurable: one map lookup per touch event.

## Design

- **Model (`gestures::Palms`):** a touch that comes down while the pen is near the screen or on it, or within 500 ms
  after it left, is a resting hand: ignored. So is any contact wider than 16 mm (a fingertip presses 8 to 12 mm, the
  side of a hand 20 mm and more). A touch keeps its verdict until it lifts. Otherwise it draws ink when "finger draws"
  is on, and passes to the platform (pan, zoom, press) when off.
- **Windows:** pen hover and contact (`WM_POINTERUPDATE`) and leaving (`WM_POINTERLEAVE`) tell the model where the pen
  is; a touch's contact width comes from `GetPointerTouchInfo`'s contact rectangle, in millimetres at the window's
  DPI. An ignored touch is swallowed (no mouse messages); a drawing one is a stroke with no pressure.
- **Setting:** `finger_draws` in `settings.toml`, the canvas's `fingerDraws` prop.

## Acceptance tests

- [x] Writing with the hand resting on a touch screen leaves no stray marks in a recorded test:
  `writing_with_the_hand_resting_leaves_no_stray_marks` (a session of pen hover and contact with a palm, the side of
  the hand, a knuckle, a touch while the hand lifts, and a fingertip later: only the last draws).
- [x] `without_finger_draws_a_finger_pans`, `a_palm_is_ignored_with_no_pen_about_and_keeps_its_verdict`.

## Open

- The session is synthetic, from how a Surface reports a resting hand; redo it from a recording (with H.1's, DD24).
- macOS and Linux hosts don't feed the model yet (their touch goes to the platform as before).
- Real-hardware behaviour is the maintainer's to check on a Surface Pro.
