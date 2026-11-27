# Ph5: 11/45 | W.10: Workspaces and library collections

**Tag:** W.10; **Priority:** P1; **Phase:** 5; **Needs:** F.6; **Code:** `ecclesia-features::workspaces_rules`, `ecclesia-store::library`, `ecclesia-store::workspace`
**Plan:** §27 entry `#### Ph5: 11/45 | W.10`.

## Problem

Readers organize long-term reading lists in library collections and active reading sessions in workspaces, with no way to convert between them.

## User story

As a reader, I want "Open collection as workspace" and "Save workspace as collection" while preserving file order, so that I can move seamlessly between library organization and active reading.

## UI sketch

- Collection context menu: "Open as workspace".
- Workspace menu / Command Palette: "Save workspace as collection…".

## Mode

All modes.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched: single SQLite transaction per conversion.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::workspaces_rules::{open_collection_as_workspace, save_workspace_as_collection}`.
- Storage: `library_collection_item.position` and `workspace_tab.position` preserve exact file order in both directions.

## Acceptance tests

- [x] `collection_and_workspace_convert_bidirectionally_preserving_order`: Converting collection -> workspace -> collection preserves exact file order.
