# Ph6: 6/13 | I.3: Ask with cited answers

**Tag:** I.3; **Priority:** P1; **Phase:** 6; **Needs:** I.2; **Code:** `crates/ecclesia-ai/src/rag.rs`, `crates/ecclesia-features/src/ai/chat.rs`
**Plan:** §27 entry `#### Ph6: 6/13 | I.3` and §12.2–§12.3.

## Problem

Unchecked LLM answers invent page numbers and quotes, breaking academic trust.

## User story

As a reader, I want to ask questions over a selection, page, document, workspace, or library and click inline `[1]` citations to jump straight to the highlighted PDF passage.

## UI sketch

- Right-hand AI chat sidebar (`ChatPanelModel`) with scope selector (`Selection`, `Page`, `Document`, `Workspace`, `Library`), `Local`/`Cloud` badge, Markdown answer view, and clickable `[1]` citation pills.
- Unverified claims are omitted from citations or clearly marked as unverified; insufficient evidence returns a refusal.

## Mode

All reading/annotation/study modes when AI is enabled.

## Speed and size impact

- Startup: panel model initializes empty with zero work until opened.
- Retrieval runs against the local SQLite hybrid index before a single provider completion call.

## Design

- `RagEngine::ask` retrieves passages inside `RagScope`, wraps them in escaped `<document_passage>` delimiters, detects prompt-injection signals, and validates every claim's `chunk_id` and `quote` via `verify_quote_in_chunk` (verbatim or normalized ligature/whitespace match).
- `ChatPanelModel::resolve_citation` maps clicked `[n]` markers to `CitationJumpTarget` (`page`, `ecclesia_mupdf::Quad` list, verbatim `quote`).

## Acceptance tests

- [x] `rag_verifies_verbatim_and_normalized_quotes_and_rejects_hallucinations`: verified quotes pass while fabricated chunks/quotes are refused.
- [x] `prompt_injection_is_detected_and_isolated_in_escaped_passages`: injected XML closing tags and override instructions are escaped and flagged.
- [x] `chat_panel_resolves_clicked_citation_to_mupdf_quads`: clicking `[1]` resolves exact page and `Quad` bounds.
