# Ph7: 1/32 | R.18: Thumbnail drag reordering

**Tag:** R.18; **Priority:** P1; **Phase:** 7; **Needs:** R.5, O.1; **Code:** `ecclesia-core::page_ops` (`plan_thumbnail_drag_reorder`, `PageOrganizer`)
**Plan:** §27 entry `#### Ph7: 1/32 | R.18`.

## Problem

Dragging one or more page thumbnails in the thumbnail sidebar or full-window grid requires deterministic index planning and undoable PDF page-tree updates even across large multi-hundred-page documents.

## User story

As a reader organizing a large PDF, I want to drag single or multi-selected page thumbnails directly to a drop target and undo or redo the move cleanly.

## UI sketch

- Thumbnail grid and sidebar show live insertion indicator lines during drag-over and move selected pages as a contiguous block to the target drop slot.
- Multi-select drag preserves relative selection order and supports standard undo/redo (`Ctrl+Z` / `Ctrl+Y`).

## Mode

Thumbnails sidebar and Page Organizer grid in Read and Organize modes.

## Speed and size impact

- Startup: zero startup cost; planning runs only on drop commit.
- Runtime: deterministic `O(N)` permutation planning (`plan_thumbnail_drag_reorder`) before applying one undoable command on `PageOrganizer`.

## Design

- **`plan_thumbnail_drag_reorder(page_count, selected_indices, insert_before)`**: Validates and deduplicates multi-selection indices, adjusts the drop slot for removed elements preceding `insert_before`, and produces the target permutation plus an inverse mapping.
- **`PageOrganizer` integration**: Executes reorder commands as atomic undoable entries, keeping `/Pages` `/Kids`, `/Count`, `/PageLabels`, and outline destination indices consistent.

## Acceptance tests

- [x] `organizer_undo_redo_and_hundred_page_drag_reorder`: multi-selected thumbnail drag reorder on a 100-page PDF matches expected page order and restores the original state on undo.
