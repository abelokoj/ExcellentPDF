# Ph7: 13/32 | O.1: Page organizer

**Tag:** O.1; **Priority:** P0; **Phase:** 7; **Needs:** R.5, K.14; **Code:** `ecclesia-core::page_ops` (`PageOrganizer`, `reorder_pages`, `rotate_pages`, `insert_blank_page`, `insert_pages_from`, `duplicate_pages`, `delete_pages`, `extract_pages`, `set_page_labels`, `read_page_labels`)
**Plan:** §27 entry `#### Ph7: 13/32 | O.1`.

## Problem

Users need full structural PDF page editing—reordering, rotating, inserting blank or external pages, duplicating, deleting, extracting, and customizing `/PageLabels`—while keeping outlines accurate and supporting multi-step undo/redo.

## User story

As a PDF editor, I want an organizer grid to rearrange, rotate, insert, duplicate, delete, extract, and renumber pages with full undo/redo and intact outline targets.

## UI sketch

- Full-window thumbnail organizer toolbar (`Rotate Left/Right`, `Insert Blank`, `Insert From PDF`, `Duplicate`, `Delete`, `Extract`, `Page Labels`, `Undo/Redo`).

## Mode

Organize mode (`PageOrganizer`).

## Speed and size impact

- Page tree mutations operate directly on parsed PDF objects with structural sharing/clean rewrite and maintain an undo stack of prior PDF snapshots/commands.

## Design

- **Command model (`PageOrganizer`, `PageCommand`)**: Executes undoable page edits (`Reorder`, `Rotate`, `InsertBlank`, `InsertFromPdf`, `Duplicate`, `Delete`, `SetPageLabels`, `DeletePageLabels`, `Crop`, `Uncrop`) with clean `undo`/`redo`.
- **Structural tree & label consistency**: Flattens/reparents `/Pages` `/Kids`, updates `/Count`, preserves and remaps `/Outlines` page references, and encodes `/Catalog` `/PageLabels` `/Nums` number trees (`PageLabelRange`, `PageLabelStyle`).

## Acceptance tests

- [x] `organizer_undo_redo_and_hundred_page_drag_reorder`: tests command execution, multi-step undo/redo, and 100-page reordering.
- [x] `page_labels_rotate_duplicate_delete_and_extract`: verifies rotation, duplication, deletion, extraction into a standalone PDF, and `/PageLabels` round-tripping in MuPDF.
