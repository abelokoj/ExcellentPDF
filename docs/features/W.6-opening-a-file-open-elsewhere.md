# Ph1: 50/60 | W.6: Opening a file that is open elsewhere

**Tag:** W.6; **Priority:** P0; **Phase:** 1; **Needs:** W.1, K.5; **Code:** `ecclesia-app::workspaces` (`view`), `ecclesia-store::workspaces`, `ecclesia-app::session`
**Plan:** §27 entry `Ph1: 50/60 | W.6` and §11.16.

## Problem

Opening a file already open in another workspace made a second, unrelated tab of it.

## User story

As a reader, I want the app to take me to the tab I already have, or open it here too if I prefer.

## UI sketch

- Settings window, Tabs, "A file open in another workspace": **Switch there** (the default) or **Open here too**
  (`open_elsewhere` = `"switch"` or `"here"`). It applies to the open dialog, files dropped on the window, and
  files from the command line or another program.

## Mode

All modes; the switcher is always in the title bar.

## Speed and size impact

- Startup: the list of workspaces (one small query) and the shown workspace's tabs, as before.
- Budgets: a workspace not shown is rows only (§11.16 step 1). Switch time not measured (K.24). Nothing to the
  installer.

## Design

- `workspaces::holding` finds the first other workspace with a tab of the file. Two tabs of one file share its
  document through the registry, as split panes do.
- shortcut: files are matched by path, ignoring case, not by identity (K.5); drops on a pane, recent files and
  `--split` launches open in place.

## Acceptance tests

- [x] `a_file_open_in_another_workspace_is_found_there`: the other workspace is found by path
- [x] `opening_elsewhere_reads_its_setting`: the setting reads "switch" or "here"
