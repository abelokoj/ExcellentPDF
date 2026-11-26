# Ph6: 5/13 | I.2: Document index for AI

**Tag:** I.2; **Priority:** P1; **Phase:** 6; **Needs:** F.3, I.1; **Code:** `crates/excellent-ai/src/index.rs`
**Plan:** §27 entry `#### Ph6: 5/13 | I.2` and §12.2.

## Problem

Long PDFs exceed model context windows and lack page/quad anchors when searched naively.

## User story

As a researcher, I want documents chunked into overlapping passage windows with page and bounding-quad metadata so AI retrieval finds exact passages quickly.

## UI sketch

- Background indexing indicator in the AI chat panel; command palette `AI: Rebuild Document Index`.
- Empty/offline states use the local SQLite FTS5 + hashed TF-IDF vector index without any network dependency.

## Mode

All modes when AI is enabled.

## Speed and size impact

- Startup: zero work at startup; `ensure_schema(&Connection)` creates tables lazily when AI retrieval runs.
- Storage: index tables (`ai_doc_meta`, `ai_chunks`, `ai_chunks_fts`) live inside `excellent-ai::index` and skip unchanged documents via SHA-256 content hashes.

## Design

- `chunk_document` splits `PageSource` (constructible from `excellent_text::PageText`) into overlapping ~512-token windows preserving `page`, `chapter`, and `CitationQuad` coordinates.
- `DocumentIndex` combines SQLite FTS5 BM25 scores, in-Rust BM25 coverage boost, and cosine similarity over 128-dim hashed TF-IDF (or provider-supplied) vectors with document, page-range, and chapter metadata filters.

## Acceptance tests

- [x] `chunking_preserves_pages_quads_and_overlap`: overlapping windows retain page indices and bounding quads.
- [x] `hybrid_sqlite_index_retrieves_relevant_pages_and_respects_filters`: hybrid search ranks target pages first and honors chapter/page filters.
- [x] `ai_eval_suite_meets_recall_citation_injection_and_consent_gates`: measures `recall@k` on the evaluation corpus (`tests/ai-eval/corpus.json`).
