# Ph6: 12/13 | I.11: Suggested highlights and tags

**Tag:** I.11; **Priority:** P2; **Phase:** 6; **Needs:** I.9; **Code:** `crates/excellent-features/src/ai/extract.rs`
**Plan:** §27 entry `#### Ph6: 12/13 | I.11` and §12.3.

## Problem

Readers want AI-assisted triage of key passages without having unreviewed highlights written into their PDFs.

## User story

As a reader, I want suggested highlights and tags shown as dismissible ghost overlays so my document stays untouched unless I explicitly accept a suggestion.

## UI sketch

- Page canvas renders translucent ghost highlight quads with floating **Accept** and **Dismiss** buttons and suggested A.17 category + tag chips.

## Mode

Read and Annotate modes when AI is enabled.

## Speed and size impact

- Startup: none. Ghost suggestions live purely in transient memory until accepted.

## Design

- `suggest_highlights_from_extractions` builds `SuggestedHighlight` items initialized to `DraftDecision::PendingReview`.
- `apply_accepted_highlights` materializes `Annotation`s (`Markup::Highlight` + `Category` + tags + K.28 `Quote`) ONLY for `DraftDecision::Accepted` items; dismissing leaves the document unchanged.

## Acceptance tests

- [x] `extraction_evidence_table_and_ghost_highlights_enforce_verification_and_consent`: pending and dismissed ghost highlights leave document annotations completely unchanged.
