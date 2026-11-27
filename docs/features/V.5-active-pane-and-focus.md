# Ph1: 41/60 | V.5: Active pane and focus

**Tag:** V.5; **Priority:** P0; **Phase:** 1; **Needs:** V.1; **Code:** `ecclesia-app::panes` (`layout`, `grid`), `ecclesia-app::shell`, `ecclesia-app::commands`
**Plan:** §27 entry `Ph1: 41/60 | V.5` and §11.15.

## Problem

With several panes, the reader must see which one the keys and commands act on, and reach any pane without the mouse.

## User story

As a keyboard user, I want to move between panes with a key and see which one is active.

## UI sketch

- The active pane has a 2-point border in the accent colour; the others a border in the border colour.
- **Go to pane 1** to **4** (Alt+1 to Alt+4; the plan's Ctrl+1 to 4 are taken by the tabs, see dev_decisions DD4),
  **Next pane** (F6), **Previous pane** (Shift+F6). Pressing on a pane makes it active.
- Every command, the tab strip, the page button and the sidebar's Pages, Outline and Bookmarks panels are the
  active pane's.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `Panes::active` is a memo over the layout and the last focused cell; each pane's focus view reports focus in.

## Acceptance tests

- [x] `panes_are_reached_in_reading_order_round_the_end`: every pane is reached in reading order, by key
