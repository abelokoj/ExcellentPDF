# Ph5: 29/45 | F.3: Full-text index

**Tag:** F.3; **Priority:** P0; **Phase:** 5; **Needs:** K.20, K.13; **Code:** `excellent-store::fts`, `excellent-features::search::global`
**Plan:** §27 entry `#### Ph5: 29/45 | F.3` and §7.4.

## Problem

Scanning PDFs from disk on every search is too slow for large libraries and misses annotations, handwriting, transcripts, and notes.

## User story

As a reader, I want a persistent full-text index covering pages, annotations, handwriting, transcripts, and notes, so that library searches are instant and jump to the exact page and quad.

## UI sketch

- Background indexing indicator in the status bar with automatic pause when scrolling or inking.
- Search hits display page number and jump straight to the matching quad or annotation.

## Mode

All modes.

## Speed and size impact

- Startup: no indexing runs at startup; triggered incrementally on file open or folder scan.
- Budgets touched: skips unchanged files via `fts_file_state` `(size, modified_at)`; cooperative `IndexControl` pause/cancel avoids UI stalls.
- Installer size: 0 bytes added (SQLite FTS5 built into bundled `rusqlite`).

## Design

- Model: `excellent-store::fts::{SourceKind, FtsEntryInput, FtsHit, replace_file_entries, needs_reindex, search_fts, search_trigram}`.
- Migration: `0008_library_fts_notes.sql` creates `fts_entry`, `fts_index` (`unicode61 remove_diacritics 2`), `fts_trigram` (`trigram`), sync triggers, and `fts_file_state`.

## Acceptance tests

- [x] `fts5_indexes_pages_annotations_and_folds_diacritics`: FTS5 indexes pages with diacritic folding, trigram substring search, stored quads, and incremental `needs_reindex` skipping.
