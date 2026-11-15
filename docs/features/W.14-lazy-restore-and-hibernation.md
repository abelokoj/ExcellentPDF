# Ph1: 52/60 | W.14: Lazy restore and hibernation

**Tag:** W.14; **Priority:** P0; **Phase:** 1; **Needs:** K.5, K.20; **Code:** `excellent-app::workspaces`, `excellent-app::session`, `excellent-app::panes`
**Plan:** §27 entry `Ph1: 52/60 | W.14` and §11.16.

## Problem

Workspaces must cost almost nothing until shown, or a reader with many of them pays at every start.

## User story

As a reader with many workspaces, I want them to cost nothing until I open them.

## UI sketch

- Nothing to see: switching shows the other workspace's panes with only each pane's shown tab opening.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- A tab not shown is a row (path, view state, pin, group); only each pane's shown tab opens a document, and no
  file is held open by a tab not shown. Writes are debounced (1 s) and made at once on switch and quit.
- Not built: the warm cache of the last workspace or two (a switch closes the other workspace's documents when no
  pane shows them), per-tab thumbnails, and the 1,000-switch leak test and §11.16 budgets (K.24).

## Acceptance tests

- [x] `a_session_comes_back_with_every_tab_at_its_page`: tabs come back as rows, each at its page
- [ ] the §11.16 budgets and 1,000 switches without a leak: **not measured**
