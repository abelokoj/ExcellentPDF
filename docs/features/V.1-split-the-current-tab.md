# Ph1: 37/60 | V.1: Split the current tab

**Tag:** V.1; **Priority:** P0; **Phase:** 1; **Needs:** K.5, K.8; **Code:** `ecclesia-app::panes` (`layout`, `grid`), `ecclesia-app::shell`, `ecclesia-app::commands`
**Plan:** §27 entry `Ph1: 37/60 | V.1` and §11.15.

## Problem

One window showed one document at a time. Comparing two places in one file (a figure and the text that cites it, a definition and its use) meant scrolling back and forth.

## User story

As a reader, I want the shown tab again in a new pane beside or under it, at the same place, so that I can read two parts of one document side by side.

## UI sketch

- **Split right** (`Ctrl+\`) and **Split down** (`Ctrl+Alt+\`), in the palette's "Split view" section; they need
  a document. The new pane shows the shown tab at its page and view, and takes the keyboard focus.
- Up to four panes, in a two-by-two grid. With every cell open, splitting does nothing.

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- Model: `panes::layout::Layout`, plain data: which of the four cells are open, the share of each row and column.
  `plan_split(side)` finds the cell for a new pane (moving the pane at that edge over when the cell is taken),
  `apply` makes the change. No UI or OS types; unit-tested.
- View: each cell has its own page canvas, tabs, page state, links, thumbnails, outline, bookmarks and edits
  (`panes::Pane`). Two panes showing one file share its document through `ecclesia_core::shared::registry()`, so
  the file is read once.
- Commands: `CommandId::Pane(PaneAction::Split(Side))`.

## Acceptance tests

- [x] `splitting_right_then_down_fills_the_grid`: splits fill the grid and stop at four
- [x] `a_pane_at_the_edge_moves_over_for_a_new_one_on_that_side`: a split at a taken edge moves the pane over
- [ ] the second pane's first frame within 50 ms with no re-parse: **not measured**; the document is shared, so it is not parsed again
