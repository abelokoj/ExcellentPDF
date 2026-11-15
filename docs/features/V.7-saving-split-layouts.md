# Ph1: 43/60 | V.7: Saving split layouts

**Tag:** V.7; **Priority:** P0; **Phase:** 1; **Needs:** V.4, R.9; **Code:** `excellent-app::session`, `excellent-store::workspace` (migration `0005_panes.sql`)
**Plan:** §27 entry `Ph1: 43/60 | V.7` and §11.15.

## Problem

A split that is gone after a restart has to be made again every day.

## User story

As a reader, I want the app to come back with the panes, their sizes and every pane's tabs as I left them.

## UI sketch

- Nothing to press: the session is saved a second after each change and at quit (R.9).

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `workspace::Panes { layout, panes }`: the layout's JSON and each cell's tabs by cell number. A session from
  before the split view comes back as one pane.

## Acceptance tests

- [x] `a_split_comes_back_with_each_pane_s_tabs`: the layout and every pane's tabs round-trip
- [x] `each_pane_keeps_its_own_tabs_and_the_layout`: the library keeps each pane's tabs and the layout
- [ ] a restart restores the exact split: **tried by hand only**
