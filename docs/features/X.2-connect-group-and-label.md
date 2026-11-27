# Ph9: 6/13 | X.2: Connect, group and label

**Tag:** X.2; **Priority:** P1; **Phase:** 9; **Needs:** X.1; **Code:** `ecclesia-features::canvas_board`
**Plan:** §27 entry `#### Ph9: 6/13 | X.2` and §11.10.

## Problem

Unconnected excerpt cards on a canvas cannot express argument structure, support/objection relationships, or thematic groupings.

## User story

As a researcher, I want to label cards with argument roles (claim, support, objection, evidence), draw labeled Bezier arrows between them, and group items into colored containers that move together cleanly.

## UI sketch

- Card role badge selector, drag-to-connect Bezier arrow handle with inline label editor, and group header bar with color swatch.

## Mode

Research, Study.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched: connector endpoints compute dynamically from card bounds.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::canvas_board::{BoardGroup, BoardConnector, ConnectorKind, ArgumentRole, CubicBezier}` and `CanvasBoard::move_group`.

## Acceptance tests

- [ ] `moving_group_moves_items_and_keeps_bezier_connectors_attached`: Translating a group moves member cards and updates attached Bezier connector endpoints automatically.
