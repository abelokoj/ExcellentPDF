# Ph5: 38/45 | F.12: Notes with backlinks and [[links]]

**Tag:** F.12; **Priority:** P1; **Phase:** 5; **Needs:** F.3, N.14; **Code:** `ecclesia-store::notes`, `ecclesia-features::notes`
**Plan:** §27 entry `#### Ph5: 38/45 | F.12`.

## Problem

Highlights inside PDFs are isolated per file unless readers can write connected Markdown notes with `[[wiki-links]]`, backlinks, and document/page anchors.

## User story

As a student or researcher, I want Markdown notes with `[[note]]` and `[[doc#page]]` links plus a live backlinks panel that updates links automatically on rename, so that my reading notes form a connected knowledge graph.

## UI sketch

- Notes panel alongside the reader with Markdown editor, clickable `[[wiki-links]]`, incoming backlinks list, and unresolved links list.

## Mode

Read, Annotate, Study.

## Speed and size impact

- Startup: notes load lazily when the Notes panel opens.
- Budgets touched: `note` and `note_link` tables keep incoming/outgoing edges indexed; note bodies index into FTS5 on save.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::notes::{Notes, BacklinkGraph, parse_links_in_markdown}`.
- Storage: `note` and `note_link` tables in `0008_library_fts_notes.sql`, managed by `ecclesia_store::notes`.

## Acceptance tests

- [x] `wiki_links_backlinks_and_renaming_update_all_references`: Extracts `[[wiki-links]]`, tracks unresolved targets, builds backlinks, and updates all referring notes when a note is renamed.
- [x] `renaming_note_updates_links_and_fts_index`: Updates both note graph and FTS5 index when a note is renamed.
