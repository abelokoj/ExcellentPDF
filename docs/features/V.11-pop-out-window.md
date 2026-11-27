# Ph3: 10/49 | V.11: Pop out to a window

**Tag:** V.11; **Priority:** P1; **Phase:** 3; **Needs:** V.3; **Code:** `ecclesia-app::windows`, `panes::pages_view`
**Plan:** §27 entry `#### Ph3: 10/49 | V.11`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Panes share one window. A reader with two screens wants a document on each, and a reader who wants a reference
beside another app wants it in a window they can place anywhere.

## User story

As a reader with more than one screen, I want to move a tab into a window of its own, so that I can put it on the
other screen and read both at once.

## UI sketch

- "Move tab to its own window or back" (Ctrl+Alt+O, the command palette) opens the shown tab in a new window, at its
  page and in its view.
- The window: the system's title bar (the file's name), a bar with the window's tabs and a "Dock" button, and the
  pages. Its tabs' and pages' shortcuts are the main window's (a window's keys fire only in it).
- Docking: the same command in that window, the Dock button, or closing the window, moves its tabs back to the main
  window's active pane. Closing its last tab closes the window.

## Mode

All modes.

## Speed and size impact

Nothing until a tab pops out. A popped-out window is one more page canvas, as a pane is.

## Design

- `crate::windows::Windows` is the window registry: each open window's `WindowRef` with its pane, a `Pane` outside
  the grid (cell `POPPED`), so it has the grid's page canvas, link, outline, bookmark, find and edit state.
- The tab moves with `TabState::take_shown` and `adopt`, as V.3 moves one between panes: page and view go with it.
  The page canvas opens files through `ecclesia_core::shared`, so a file open in both windows has one `Document`.
- The page canvas view is shared: `panes::pages_view` takes the events its host hears first (the grid: focus, sync,
  open elsewhere, full screen; the window: a Ctrl+click link goes there, as the window has one pane).
- NSH hosts a native component in a window of its own and gives it back to the main window when that window closes.
- The session keeps the windows' tabs with the active pane's (`keep_popped`), so a quit with windows open loses
  nothing; a workspace switch closes them after the session saved them with the workspace shown until then.
- `nsh.json` grants `windows` (normal windows only).

## Acceptance tests

- [x] `popped_out_tabs_are_kept_with_the_active_pane_s`: the session folds a window's tabs into the active pane's,
  pages kept, the shown tab still shown.
- [ ] Tried by hand: a popped-out tab keeps its page and view; an edit in one window shows in the other; docking and
  closing bring every tab back.

## Open

- Not tried by hand.
- Docking is a command, a button or closing: NSH 0.5 has no drag between windows, so the plan's drag waits for it.
- The shown tab pops out, not its group.
- The window has no sidebar, find bar, palette or dialogs of the main window; its tab-list button does nothing.
- A Ctrl+click link goes there in that window (one pane) rather than to another pane.
- Pen contact doesn't switch modes there: the modes live in the main window's context.
