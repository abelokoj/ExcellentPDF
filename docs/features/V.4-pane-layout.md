# Ph1: 40/60 | V.4: Pane layout

**Tag:** V.4; **Priority:** P0; **Phase:** 1; **Needs:** V.3; **Code:** `excellent-app::panes` (`layout`, `grid`), `excellent-app::shell`, `excellent-app::commands`
**Plan:** §27 entry `Ph1: 40/60 | V.4` and §11.15.

## Problem

Side by side suits two portrait pages; stacked suits wide slides; four documents need a grid.

## User story

As a reader, I want panes side by side, stacked or in a grid of four, and to size them by dragging the divider.

## UI sketch

- Dividers between panes: drag to resize (a pane keeps at least 160 points), double-click to make the panes of
  that row or column the same size. **Make panes the same size** (Ctrl+Alt+0) evens them all.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `Layout` is plain data with `to_json` / `from_json`; sizes are shares of the row and column, set from NSH's
  resizable panel groups at the end of a drag.

## Acceptance tests

- [x] `a_layout_round_trips_through_json`: layouts are plain data that round-trip through JSON
- [x] `evening_out_gives_every_pane_the_same_share`: equalizing gives every pane the same share
