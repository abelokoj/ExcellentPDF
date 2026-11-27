# Ph5: 32/45 | F.6: Library: collections, tags, read status

**Tag:** F.6; **Priority:** P1; **Phase:** 5; **Needs:** K.20; **Code:** `ecclesia-store::library`, `ecclesia-features::library`
**Plan:** §27 entry `#### Ph5: 32/45 | F.6`.

## Problem

Readers with many PDFs need structured organization beyond filesystem folders: nested collections, cross-cutting tags, and reading status tracking.

## User story

As a reader, I want nested collections, tags, and `Unread` / `Reading` / `Finished` status with progress on every document, so that I can organize and filter a large library effortlessly.

## UI sketch

- Library view with collection tree, tag filter chips, read-status filter, search bar, and paginated `DataTable` / cover grid.

## Mode

All modes.

## Speed and size impact

- Startup: library tables are queried only when opening the library view or home view.
- Budgets touched: indexed SQLite queries with `limit`/`offset` pagination scale smoothly to 50,000+ rows.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::library::{Library, CollectionNode, build_collection_tree}`.
- Storage: `library_item`, `library_collection`, `library_collection_item`, `library_file_tag` in `0008_library_fts_notes.sql`.

## Acceptance tests

- [x] `library_items_tags_and_status_round_trip`: Persists and filters library items, tags, and `ReadStatus` transitions.
- [x] `nested_collections_preserve_file_order`: Persists nested collections and preserves explicit item order.
- [x] `library_collections_tags_and_tree_work_end_to_end`: End-to-end collection tree, tags, and status updates via `Library`.
