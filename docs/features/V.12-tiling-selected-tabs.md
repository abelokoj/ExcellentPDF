# Ph5: 10/45 | V.12: Tiling selected tabs

**Tag:** V.12; **Priority:** P2; **Phase:** 5; **Needs:** V.4; **Code:** `ecclesia-app::{panes, tabs, commands}`
**Plan:** §27 entry `#### Ph5: 10/45 | V.12`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Comparing two to four open documents currently requires splitting panes manually and moving tabs one by one, and returning those documents back into a single tab group afterwards takes several separate close/move actions.

## User story

As a researcher comparing multiple papers, I want to select several tabs and tile them into a pane grid in one action—and untile them back into one group when finished—so that side-by-side comparison is effortless.

## UI sketch

- Multi-select tabs in the tab strip (ring selection indicator on selected tabs).
- **Tile selected tabs** command (command palette and tab bar action): arranges up to 4 selected tabs into a balanced pane grid (`Layout`), preserving each tab's page and view state.
- **Untile tabs** command: collapses the tiled panes back into a single pane and restores the tabs together in one tab group.
- Accessible names on every tab selection state and tile/untile command; full keyboard access via the command palette.

## Mode

Every mode. Visible when multiple tabs are selected or a tiled grid is active.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched: shares the existing `Layout` grid and shared `Document` registry (`K.5`), avoiding duplicate parsing.
- Installer size: pure Rust layout transition in `ecclesia-app::panes`.

## Design

- Model (`ecclesia-app::panes::layout`, `ecclesia-app::panes`): deterministic tile-grid layout mapping 2–4 selected tabs into pane cells and recording the pre-tile grouping so untile restores the tabs in order.
- View (`ecclesia-app::tabs`, `ecclesia-app::panes::grid`): renders multi-selection highlight on tabs and delegates grid resizing to `V.4`.
- Commands (`ecclesia-app::commands`): palette entries for tiling selected tabs and untiling back to a single pane.

## Acceptance tests

- [x] `tile_and_untile_round_trip`: tiling selected tabs into a grid and untiling returns them cleanly to one group.
- [x] `panes::layout::tests`: layout grid splits, merges, and equalizes pane shares deterministically.
- [ ] Tried by hand across 2, 3, and 4 selected tabs in the running app.

## Open

- Implemented with unit tests; awaiting maintainer hand-acceptance in the running app.
