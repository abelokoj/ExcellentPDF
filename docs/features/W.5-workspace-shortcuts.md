# Ph1: 49/60 | W.5: Workspace shortcuts

**Tag:** W.5; **Priority:** P0; **Phase:** 1; **Needs:** W.1, K.16; **Code:** `excellent-app::workspaces` (`view`), `excellent-store::workspaces`, `excellent-app::session`
**Plan:** §27 entry `Ph1: 49/60 | W.5` and §11.16.

## Problem

Switching through a list is slow for a reader who goes back and forth all day.

## User story

As a keyboard user, I want a key for each of my first workspaces and for the next and previous one.

## UI sketch

- **Go to workspace 1** to **9** (Ctrl+Alt+1 to 9), **Next workspace** (Ctrl+Alt+PageDown), **Previous
  workspace** (Ctrl+Alt+PageUp), round the ends. All are in the command table, so `settings.toml` can rebind them.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `WorkspaceAction::Go(n)` and `Cycle(step)`.

## Acceptance tests

- [x] `workspaces_cycle_round_the_ends`: next and previous go round the ends
- [x] `only_window_commands_are_offered_without_a_document`: commands that need a tab wait for one
