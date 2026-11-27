# Ph1: 47/60 | W.3: Edit and delete workspaces

**Tag:** W.3; **Priority:** P0; **Phase:** 1; **Needs:** W.1; **Code:** `ecclesia-app::workspaces` (`view`), `ecclesia-store::workspaces`, `ecclesia-app::session`
**Plan:** §27 entry `Ph1: 47/60 | W.3` and §11.16.

## Problem

A workspace has to be renamed when a project changes, and removed when it ends.

## User story

As a reader, I want to rename, recolour, give an icon to, reorder and delete workspaces.

## UI sketch

- **Edit workspace**: a dialog with the name, eight icons (pressing the chosen one again removes it), the eight tab
  group colours, Move up / Move down, Delete and Done. Changes apply as they are made.
- Delete asks: "Delete Thesis?" with **Move tabs and delete** (to the workspace before it) or **Close tabs and
  delete**. The last workspace has no Delete.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `workspaces::restyle`, `reorder`, `delete` (refuses the last). Edits are saved within 2 s and whenever a pane
  stops showing a file (K.15), so deleting never loses work and needs no further prompt.
- Not built: reordering by drag; Move up / Move down and their commands do it.

## Acceptance tests

- [x] `a_deleted_workspace_gives_way_to_the_one_before_it`: the tabs go to the workspace before
- [x] `a_workspace_moves_one_place_and_stops_at_the_ends`: reordering moves one place
- [x] `workspaces_are_made_listed_in_order_restyled_and_deleted`: the last workspace is never deleted
