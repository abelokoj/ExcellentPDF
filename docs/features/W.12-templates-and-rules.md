# Ph5: 12/45 | W.12: Templates and rules

**Tag:** W.12; **Priority:** P2; **Phase:** 5; **Needs:** W.2; **Code:** `excellent-features::workspaces_rules`
**Plan:** §27 entry `#### Ph5: 12/45 | W.12`.

## Problem

Setting up recurring project/course workspaces and manually switching workspaces when opening files from specific folders adds friction.

## User story

As a student or researcher, I want reusable workspace templates (like "Course": syllabus, textbook, notebook) and folder-based auto-routing rules, so that new workspaces and opened files land in the right place automatically.

## UI sketch

- "New workspace from template…" picker with built-in "Course" preset and custom templates.
- Workspace settings rules table mapping folder prefixes to target workspaces.

## Mode

All modes.

## Speed and size impact

- Startup: rules are evaluated only when opening a file.
- Budgets touched: O(rules) in-memory normalized prefix match, sub-millisecond.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::workspaces_rules::{WorkspaceTemplate, TemplateItem, RoutingRule, instantiate_template, match_rule, route_file}`.
- Storage: `workspace_template` and `workspace_rule` tables in `0008_library_fts_notes.sql`.

## Acceptance tests

- [x] `templates_and_path_routing_rules_route_newly_opened_files`: Instantiates the `"Course"` preset template and routes newly opened files matching a folder rule to the target workspace.
