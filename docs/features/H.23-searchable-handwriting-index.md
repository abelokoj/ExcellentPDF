# Ph5: 18/45 | H.23: Searchable handwriting index

**Tag:** H.23; **Priority:** P0; **Phase:** 5; **Needs:** H.9, F.3; **Code:** `ecclesia-features::handwriting::index`, `ecclesia-features::handwriting::recognize`
**Plan:** §27 entry `#### Ph5: 18/45 | H.23: Searchable handwriting index`.

## Problem

Handwritten lecture notes in notebooks and PDFs are invisible to text search unless recognized and indexed.

## User story

As a student with dozens of handwritten notebook pages, I want my handwriting indexed automatically in the background so searching a word highlights the matching strokes across any page.

## UI sketch

- Search bar / Global Search (`F.3`) queries `HandwritingIndex::search`.
- Matching hits render highlight boxes (`HandwritingHit::bbox`) over the corresponding `stroke_ids` on the page canvas.

## Mode

All modes; dirty pages queue automatically on pen-up in Write mode.

## Speed and size impact

- Startup: zero cost; dirty pages queue in `PendingPageQueue` and index at idle priority.
- Storage: SQLite FTS5 (`handwriting_words` + `handwriting_fts`) using existing `rusqlite`.

## Design

- **Model (`ecclesia-features::handwriting::index`, `recognize`):**
  - Cross-platform `Recognizer` trait supporting `WindowsInkAnalysis`, `AppleVision`, `MlKitDigitalInk`, `OnnxPack`, and `OfflineStrokeRecognizer` (deterministic offline geometric stroke-feature + lexicon matcher).
  - `HandwritingIndex` stores recognized words with `stroke_ids_json` and `PageRect` bounding boxes in SQLite FTS5.

## Acceptance tests

- [x] `handwriting::index::tests::fts5_index_finds_handwritten_word_and_returns_stroke_ids`: verifies indexing recognized words into FTS5 and retrieving exact `stroke_ids` and `bbox`.
- [x] `handwriting::recognize::tests::recognizes_simple_printed_letters_and_preserves_lines`: verifies offline stroke-feature + lexicon recognition across multiple lines.
