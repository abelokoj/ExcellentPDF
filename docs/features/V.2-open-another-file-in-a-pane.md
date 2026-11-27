# Ph1: 38/60 | V.2: Open another file in a pane

**Tag:** V.2; **Priority:** P0; **Phase:** 1; **Needs:** V.1, R.11; **Code:** `ecclesia-app::panes` (`layout`, `grid`), `ecclesia-app::shell`, `ecclesia-app::commands`
**Plan:** §27 entry `Ph1: 38/60 | V.2` and §11.15.

## Problem

Two different documents (a paper and its supplement) could only be shown one after the other.

## User story

As a reader, I want to open a file straight into a new pane, from the open dialog or by dropping it on an edge of the page area, so that two documents sit side by side.

## UI sketch

- **Open files in a split** (Ctrl+Shift+O): the open dialog; the first file picked opens in a new pane on the right,
  the rest as its tabs.
- While a file is dragged over the window, four bands light up along the page area's edges ("Drop to open above",
  below, on the left, on the right); the band under the pointer shows in the accent colour. Dropping on one opens the
  files in a new pane on that side. Dropping on a pane opens them in that pane; anywhere else, in the active pane.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `Panes::open_in_split(side, paths)`; the edge bands are `panes::grid::edge_bands`, shown while
  `FileDrag` (a count of drag enters, with a 150 ms grace on leave) is on.
- Not built: "Open in split" in the recent-files panel and the library (the library is F.1); listed as open.

## Acceptance tests

- [ ] dropping a file on the right edge opens it in a new right pane: **tried by hand only**
