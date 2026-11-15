# Ph1: 27/60 | R.19: Full-screen reading and focus mode

**Tag:** R.19; **Priority:** P1; **Phase:** 1; **Needs:** R.1; **Code:** `excellent-features::read::focus`,
`excellent-canvas-win::canvas::reading`, `excellent-app::{full_screen, reading}`
**Plan:** §27 entry `Ph1: 27/60 | R.19`.

## Problem

The chrome and the rest of the page compete with the line being read.

## User story

As a reader of long papers, I want the pages alone on the screen and the paragraph I am reading lit, so that nothing
else pulls at my eyes.

## UI sketch

Palette, group **Reading**: Full screen (`Ctrl+Shift+L`; F11 too, which the platform handles), Focus mode (no key),
Leave full screen and reading modes (`Esc`, while the pages have the keyboard).

- Full screen: the title bar, the docks and the document's tab bar hide; the pages fill the screen. The pointer at
  the top edge brings the title bar back, pushing the pages down (the canvas is a window of its own, so nothing can
  lie over it); leaving the bar hides it again. Leaving full screen opens the docks as they were.
- Focus mode: the page outside the reading line's paragraph (or line, setting `focus_scope`) is dimmed with the
  page-area colour. It follows the reading line: the arrow keys move it (R.20), and the pointer when the ruler is on.

## Mode

Every reading mode. Off by default.

## Speed and size impact

- Startup: nothing; the props are three small values.
- Focus mode reads the text of the pages shown and one either side (K.13), on the workers.
- Drawing: one geometry fill per frame while it is on.

## Design

Model (`read::focus`): `Scope { Paragraph, Line }`, `lit(layout, line, scope)` gives the box to keep lit, and
`reveals(y, scale)` says whether the pointer is at the top edge (6 px at 96 dpi). The canvas draws the dim with
`Gfx::dim_outside` and reports `edge` (`{at}`); the app's `FullScreen` follows the window's mode (so F11 works the same
way), closes and restores the docks, and hides the title bar. Settings: `focus_scope` (`"paragraph"`).

## Acceptance tests

- [x] `focus_mode_lights_the_reading_lines_paragraph_or_the_line_alone`, `the_title_bar_comes_back_at_the_top_edge`.
- [ ] Esc leaves both modes: `leaveModes` turns off focus mode, the ruler and auto-scroll, and the app leaves full
  screen; not yet tried by hand.
- [ ] Focus mode follows the keyboard caret: it follows the reading line the arrow keys move. There is no text caret
  yet (K.13's caret browsing is not built); revisit when it is.
