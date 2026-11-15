# Ph1: 45/60 | W.1: Workspace switcher

**Tag:** W.1; **Priority:** P0; **Phase:** 1; **Needs:** W.14; **Code:** `excellent-app::workspaces` (`view`), `excellent-store::workspaces`, `excellent-app::session`
**Plan:** §27 entry `Ph1: 45/60 | W.1` and §11.16.

## Problem

All open files shared one tab strip, so a thesis, a course and the tax papers crowded each other.

## User story

As a reader with several projects, I want named sets of tabs and one click to go from one to another.

## UI sketch

- Title bar, left of the tab strip: the switcher button, with the workspace's colour dot, icon and name
  (screen readers: "Workspace Thesis: Switch workspace").
- Pressing it opens the workspace list (a dialog, as the tab list is: a popover would fall behind the page canvas):
  every workspace with its icon and tab count, the shown one checked, Ctrl+Alt+1 to 9 beside the first nine; then
  New workspace, New workspace from all tabs, Move tab to a new workspace, Move tab to workspace, Edit workspace.
- The palette's "Workspaces" section has every workspace command, "Switch workspace" first.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `Workspaces` keeps the list and the shown id; `switch` saves the shown panes at once (`session::Keeper`), loads
  the other's (`session::load`) and shows them (`Panes::show`) in one batch, so the saver never pairs one
  workspace's id with the other's tabs.
- Not built: the left rail entry (the rail comes later), the dirty dot (edits are saved within 2 s, K.15), and the
  tab strip tinted with the workspace colour.

## Acceptance tests

- [x] `workspaces_are_made_listed_in_order_restyled_and_deleted`: the list is read in order with tab counts
- [x] `the_palette_lists_every_command_and_runs_one`: the palette runs commands, workspaces included
- [ ] switching by mouse, keyboard and palette: **tried by hand only**
