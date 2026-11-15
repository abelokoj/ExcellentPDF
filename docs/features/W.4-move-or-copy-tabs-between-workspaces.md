# Ph1: 48/60 | W.4: Move or copy tabs between workspaces

**Tag:** W.4; **Priority:** P0; **Phase:** 1; **Needs:** W.1; **Code:** `excellent-app::workspaces` (`view`), `excellent-store::workspaces`, `excellent-app::session`
**Plan:** §27 entry `Ph1: 48/60 | W.4` and §11.16.

## Problem

A file opened in the wrong workspace had to be closed and opened again in the right one.

## User story

As a reader, I want to send the tab I am reading to another workspace, keeping my place in it.

## UI sketch

- **Move tab to workspace** and **Copy tab to workspace**: the workspace list without the shown one; Return sends the
  tab there, shown in its active pane at its page and view.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `session::add_to` adds the tab to the other workspace's saved panes in one library write. Closed-tab lists are
  per workspace already (§11.16 storage).
- Not built: the tab context menu (popups fall behind the canvas) and dragging a tab onto the switcher.

## Acceptance tests

- [x] `a_tab_moved_to_another_pane_keeps_its_page_and_view`: a taken tab keeps its page and view
- [ ] a moved tab keeps its View state in the other workspace: **tried by hand only**
