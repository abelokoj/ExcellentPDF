# Ph1: 46/60 | W.2: Create a workspace

**Tag:** W.2; **Priority:** P0; **Phase:** 1; **Needs:** W.1; **Code:** `ecclesia-app::workspaces` (`view`), `ecclesia-store::workspaces`, `ecclesia-app::session`
**Plan:** §27 entry `Ph1: 46/60 | W.2` and §11.16.

## Problem

A new project needs a clean strip, or the tabs already open moved into one.

## User story

As a reader, I want to start a workspace empty, from all my tabs, or from the tab I am reading.

## UI sketch

- **New workspace** (Ctrl+Alt+N), **New workspace from all tabs**, **Move tab to a new workspace**. The new
  workspace is shown at once, named "Workspace n" with the next colour, and its edit dialog opens to name it.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- The tabs leave at their page and view (`Panes::take_every_tab`, `TabState::take_shown`) and are adopted by
  the new workspace's active pane.
- Not built: multi-select of tabs (R.12 has none yet), so "from selected tabs" is the shown tab.

## Acceptance tests

- [x] `workspaces_are_made_listed_in_order_restyled_and_deleted`: a new workspace is added after the others
- [x] `a_closing_pane_gives_up_every_tab_the_shown_one_last`: every tab leaves, the shown one last
