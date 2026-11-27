# Ph5: 31/45 | F.5: Saved searches and smart folders

**Tag:** F.5; **Priority:** P1; **Phase:** 5; **Needs:** F.4, F.6; **Code:** `ecclesia-store::fts`, `ecclesia-features::search::query`, `ecclesia-features::search::global`
**Plan:** §27 entry `#### Ph5: 31/45 | F.5`.

## Problem

Readers repeatedly type complex filter queries (such as recent unread method highlights) to monitor their library.

## User story

As a researcher, I want to save named search queries as smart folders in my library tree with live match counts, so that new matching documents and annotations appear automatically.

## UI sketch

- "Save search…" action next to the global search box.
- Smart folders section in the library tree showing each saved search name and live match count.

## Mode

All modes.

## Speed and size impact

- Startup: smart folder metadata loads lazily when the library sidebar is shown.
- Budgets touched: backed by indexed FTS5 + SQL queries (`smart_folder_counts`).
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::search::query::{create_saved_search, list_saved_searches}` and `ecclesia-features::search::global::smart_folder_counts`.
- Storage: `saved_search` table in `0008_library_fts_notes.sql` accessed via `ecclesia_store::fts::{save_search, list_searches, rename_search, delete_search}`.

## Acceptance tests

- [x] `saved_searches_crud_round_trips`: Saving, listing, renaming, and deleting saved searches round-trips in SQLite.
- [x] `streams_corpus_pdf_search_and_updates_smart_folders`: Smart folders compute live hit counts over indexed content.
