# Ph9: 5/13 | X.1: Excerpts on an infinite canvas

**Tag:** X.1; **Priority:** P1; **Phase:** 9; **Needs:** K.28, N.5; **Code:** `ecclesia-features::canvas_board`
**Plan:** §27 entry `#### Ph9: 5/13 | X.1` and §11.10.

## Problem

Linear note files make it difficult to visually spatialize excerpts, figures, and notes from multiple PDFs side-by-side while keeping one-click jump-back links to exact source regions.

## User story

As a researcher, I want an infinite spatial canvas board (`.exboard`) where I can drag text excerpts, region crops, notes, and images with pan/zoom and jump back to the exact PDF location even after a document is re-typeset.

## UI sketch

- Side-by-side Canvas Board workspace pane alongside the PDF viewer with pan/zoom viewport, drag-and-drop excerpt creation, and "Jump to Source" action on every excerpt card.

## Mode

Research, Study.

## Speed and size impact

- Startup: canvas board and spatial index initialize lazily when opening an `.exboard`.
- Budgets touched: uniform `SpatialGridIndex` answers viewport queries over 1,000+ cards in O(1) amortized time.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::canvas_board::{CanvasBoard, BoardCard, CardKind, DocumentAnchor, Viewport}` + `canvas_board::spatial::SpatialGridIndex`.
- Re-anchoring: `DocumentAnchor::relocate_in_text` delegates to `ecclesia_core::anchor::locate` (K.28).

## Acceptance tests

- [ ] `spatial_index_handles_1500_cards_and_relocates_retypeset_excerpt`: Inserts 1,500 excerpt cards, queries viewport subset accurately, and re-locates an excerpt on re-typeset source text.
