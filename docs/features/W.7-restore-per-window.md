# Ph1: 51/60 | W.7: Restore per window

**Tag:** W.7; **Priority:** P0; **Phase:** 1; **Needs:** R.9; **Code:** `ecclesia-app::workspaces` (`view`), `ecclesia-store::workspaces`, `ecclesia-app::session`
**Plan:** §27 entry `Ph1: 51/60 | W.7` and §11.16.

## Problem

After a restart the app went back to the first workspace, not the one the reader left.

## User story

As a reader, I want the app to reopen on the workspace I was in.

## UI sketch

- Nothing to press: the window reopens on its last workspace, with its panes and tabs.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `workspaces::shown` / `set_shown` keep the window's workspace in `meta`. shortcut: one window, one key; W.8's
  windows each get a key of their own.

## Acceptance tests

- [x] `the_shown_workspace_is_remembered_until_it_goes`: the shown workspace is remembered
- [ ] two windows restore two workspaces: **waits for W.8** (one window until then)
