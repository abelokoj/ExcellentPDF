# Ph9: 1/13 | W.13: Sync workspaces

**Tag:** W.13; **Priority:** P2; **Phase:** 9; **Needs:** Y.1; **Code:** `ecclesia-features::sync::folder`
**Plan:** §27 entry `#### Ph9: 1/13 | W.13` and §11.16.

## Problem

Workspaces created on one computer (such as a Windows workstation) do not automatically appear on another computer (such as a macOS laptop), and absolute file paths differ across operating systems.

## User story

As a multi-device reader, I want my workspaces, split panes, tab groups, and pinned tabs to sync through my sync folder with paths mapped per device so that opening EcclesiaPDF on another machine shows the same workspaces.

## UI sketch

- Sync status indicator in Workspace Switcher and Settings -> Sync ("Sync workspaces" toggle and Device Root Path Mapping editor).
- Accessible labels for root alias mappings (e.g. `${DOCS}` -> local folder).

## Mode

All workspace-enabled modes (Read, Annotate, Study, Research).

## Speed and size impact

- Startup: workspace oplogs sync asynchronously in the background after initial workspace load.
- Budgets touched: zero impact on cold start (< 300 ms).
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::sync::folder::{DevicePathMap, export_workspaces_to_sync, import_workspaces_from_sync}` and `SyncedWorkspace` CRDT operations.
- Storage: reads and writes `workspace`, `workspace_pane`, `workspace_tab`, and `tab_group` via `ecclesia_store::{workspace, workspaces}` through `.ecclesia/workspaces/<device-id>.oplog`.

## Acceptance tests

- [ ] `workspace_sync_maps_paths_across_windows_and_macos`: A workspace created on a Windows store exports via `${DOCS}` path tokens and materializes on a macOS store with mapped POSIX paths.
