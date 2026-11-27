# Ph6: 10/13 | I.9: Claim, method, finding and limitation extraction

**Tag:** I.9; **Priority:** P1; **Phase:** 6; **Needs:** I.3, A.17; **Code:** `crates/ecclesia-features/src/ai/extract.rs`
**Plan:** §27 entry `#### Ph6: 10/13 | I.9`.

## Problem

Synthesizing research papers requires manually locating claims, methods, samples, findings, and limitations across pages.

## User story

As a researcher, I want AI to extract structured claims, methods, findings, and limitations with verified verbatim quotes so I can turn accepted ones into semantic annotations (A.17).

## UI sketch

- AI panel tab / palette: **Extract Paper Structure**; lists grouped items with page badges and **Add as Category Annotation** actions.

## Mode

Read and Annotate modes when AI is enabled.

## Speed and size impact

- Startup: none. Runs on demand with JSON-schema structured output.

## Design

- `extract_structured_items` enforces a strict JSON Schema (`claim`, `method`, `sample`, `finding`, `limitation`), verifies every quote against its `chunk_id` using `verify_quote_in_chunk`, rejects unverified items (`unverified_rejected`), and maps each `ExtractionKind` to `annotate::categories::Category` via `ExtractedItem::to_annotation`.

## Acceptance tests

- [x] `extraction_evidence_table_and_ghost_highlights_enforce_verification_and_consent`: every extracted item carries a verified quote and unverified items are rejected.
