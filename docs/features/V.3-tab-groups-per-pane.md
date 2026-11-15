# Ph1: 39/60 | V.3: Tab groups per pane

**Tag:** V.3; **Priority:** P0; **Phase:** 1; **Needs:** V.1, R.12; **Code:** `excellent-app::panes` (`layout`, `grid`), `excellent-app::shell`, `excellent-app::commands`
**Plan:** §27 entry `Ph1: 39/60 | V.3` and §11.15.

## Problem

Panes are only useful if each keeps its own set of tabs and a tab can go from one to another.

## User story

As a reader, I want each pane to have its own tabs, and to move a tab to the other pane without losing my place in it.

## UI sketch

- The title bar's tab strip is the active pane's. With more than one pane, each pane has a slim header with its
  file's name and a close button.
- **Move tab to the next pane** (Ctrl+Alt+Right) and **to the previous pane** (Ctrl+Alt+Left). With one pane, the
  tab moves into a new pane on that side.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `TabState::take_shown` and `adopt`: the tab leaves at its page and view (`Closed`), and arrives as it was.
- Not built: dragging a tab between panes with the mouse (R.12's mouse drag is not built either); the commands cover
  "Move to other group".

## Acceptance tests

- [x] `a_tab_moved_to_another_pane_keeps_its_page_and_view`: moving a tab keeps its view state
