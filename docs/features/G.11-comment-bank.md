# Ph7: 8/32 | G.11: Comment bank

**Tag:** G.11; **Priority:** P1; **Phase:** 7; **Needs:** A.4; **Code:** `excellent-features::grading` (`CommentBank`, `CommentBankItem`)
**Plan:** §27 entry `#### Ph7: 8/32 | G.11`.

## Problem

Teachers write the same feedback remarks across dozens of assignments and need a searchable, categorized comment bank reusable across courses and sharable as JSON.

## User story

As a teacher, I want a reusable comment bank with keyboard quick-insert, tags, optional point deltas, usage ordering, and JSON export/import so feedback stays fast and consistent.

## UI sketch

- Searchable comment drawer in the Grading panel with category filters, shortcut bindings, and JSON import/export buttons.

## Mode

Grading and Annotate modes.

## Speed and size impact

- Sub-millisecond substring search across in-memory items; serialized via compact JSON.

## Design

- **`CommentBank` & `CommentBankItem`**: Manages categorized reusable comments (`id`, `label`, `body`, `category`, `points_delta`, `shortcut`, `usage_count`) with insertion/search helpers (`search`, `record_use`) and versioned `to_json`/`from_json` round-trips.

## Acceptance tests

- [x] `grading_stamps_rubric_recalculation_and_comment_bank_round_trip`: verifies comment bank search, usage counting, and JSON serialization round-trip.
