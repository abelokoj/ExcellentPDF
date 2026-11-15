# Ph1: 56/60 | U.1: Keyboard access and focus

**Tag:** U.1; **Priority:** P0; **Phase:** 1; **Needs:** K.16; **Code:** everywhere (`excellent-app`)
**Plan:** §27.18 entry `Ph1: 56/60 | U.1`.

## Problem

A reader who can't use a mouse, or doesn't want to, must be able to do everything: open, read, move, find, close.
They need to see where the keyboard is.

## User story

As a keyboard user, I want to reach every command and control from the keyboard, in a sensible order, and always
see which control has the focus.

## UI sketch

- Every command is in the command palette (**Ctrl+K**), and most have a shortcut of their own (K.16's table, which
  the palette shows beside each name).
- **Tab** and **Shift+Tab** move through the controls in reading order: title bar, then the docks, then dialogs over
  them; arrow keys move inside lists and segment groups (NSH's roving focus).
- The control with the keyboard focus has a ring in the theme's `ring` colour (2 px; 1 px on small segments).
- Dialogs keep the focus inside them; **Esc** closes them and gives the focus back.

## Mode

All modes.

## Speed and size impact

None: styles and tests only.

## Design

- Focus order is NSH's: tree order, which the shell builds in reading order (K.16's layout), with overlays after
  their trigger. No view sets a tab index of its own.
- Visible focus: every pressable has a `focus_visible` ring. The audit for U.1 added it where it was missing: the
  page button (`goto`), the link dialog's buttons (`links`), page thumbnails (`thumbnails`), and the outline's rows
  and "Add an entry" button (`outline`).
- Commands: K.16's table puts every command in the palette, so each is reachable by keys even without a shortcut;
  `the_palette_lists_every_command_and_runs_one` (T6) checks the palette runs one.
- The page canvas takes the keys of reading (K.8) itself; its own focus is the canvas window's.
- Not built: a focus check per view in the headless driver (NSH 0.5's `Screen` has no public "focused" query), so
  the tests check what keys do, not where the ring is; Annotate and Notebook modes come with their features (A.*,
  N.*) and bring their own keyboard tests.

## Acceptance tests

- [x] `a_document_is_read_from_the_keyboard_alone` (open, the go-to box opened and closed, find, close; headless lays out no pages, so going to one is left to
  `read::goto`'s tests)
- [x] `the_palette_lists_every_command_and_runs_one`
- [x] `shortcuts_toggle_the_docks`
- [x] `the_window_works_at_double_scaling` (U.4; the palette from the keyboard at 640 × 400)
- [ ] the core tasks of each mode can be done without a mouse: Read mode is covered; Annotate and Notebook modes
  don't exist yet; **not tried by hand** with a screen reader
