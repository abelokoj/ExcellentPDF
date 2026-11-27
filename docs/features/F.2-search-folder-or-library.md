# Ph5: 28/45 | F.2: Search a folder or the library

**Tag:** F.2; **Priority:** P0; **Phase:** 5; **Needs:** F.3; **Code:** `ecclesia-features::search::global`, `ecclesia-store::fts`
**Plan:** §27 entry `#### Ph5: 28/45 | F.2` and §13.1.

## Problem

Readers with hundreds of PDFs in a project folder or library cannot search across all of them at once without opening each document individually.

## User story

As a researcher, I want to search across a chosen folder or my whole library and see hits stream in immediately, so that I find passages across papers without waiting for every unindexed file.

## UI sketch

- Search sidebar / Command Palette entry "Search library or folder…" (`Ctrl+Shift+F`).
- Shows indexed hits from `library.db` immediately, grouped by file and page with highlighted snippets, followed by streamed hits as unindexed files are scanned in the background.
- Empty, loading/indexing progress bar with pause/cancel, and error states with accessible names.

## Mode

All modes.

## Speed and size impact

- Startup: nothing loads at startup; SQLite FTS5 tables are queried only when global search opens.
- Budgets touched: first indexed hits return well under 100 ms; background folder scanning uses cooperative pause (`IndexControl`).
- Installer size: 0 bytes added (uses bundled `rusqlite` FTS5).

## Design

- Model: `ecclesia-features::search::global::search_indexed`, `search_folder_streaming`, and `IndexControl`.
- View: NSH search results list (pending UI wiring).
- Storage: `ecclesia-store::fts` (`fts_entry`, `fts_index`, `fts_file_state`).

## Acceptance tests

- [x] `streams_corpus_pdf_search_and_updates_smart_folders`: Indexed FTS5 hits return immediately and unindexed files stream hits as they are indexed.
