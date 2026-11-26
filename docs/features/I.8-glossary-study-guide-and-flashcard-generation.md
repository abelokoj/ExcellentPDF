# Ph6: 9/13 | I.8: Glossary, study guide and flashcard generation

**Tag:** I.8; **Priority:** P1; **Phase:** 6; **Needs:** I.3, S.1; **Code:** `crates/excellent-features/src/ai/study.rs`
**Plan:** §27 entry `#### Ph6: 9/13 | I.8` and §12.3.

## Problem

Turning dense chapters into flashcards and glossaries by hand is slow, yet silent AI card creation pollutes spaced-repetition decks.

## User story

As a student, I want AI to draft glossary entries, study guides, and flashcards with page citations so I can review and accept only the items I want in my deck.

## UI sketch

- Study panel & palette: **Generate Study Materials**; shows a review list with **Accept** / **Dismiss** controls per item.
- Pending or dismissed items never enter `study::cards`.

## Mode

Study and Read modes when AI is enabled.

## Speed and size impact

- Startup: none. Drafts live in memory until explicitly accepted into `excellent_store::study`.

## Design

- `generate_study_materials` generates `GlossaryEntryDraft`, `StudyGuideSectionDraft`, and `FlashcardDraft` (supporting basic and `{{c1::...}}` cloze cards), verifies every citation against source chunks, and initializes all items to `DraftDecision::PendingReview`.
- `StudyDraftBatch::into_accepted_flashcards` bridges ONLY `DraftDecision::Accepted` items into `crate::study::cards::Flashcard` with K.28 `SourceAnchor`s.

## Acceptance tests

- [x] `study_drafts_require_explicit_acceptance_before_becoming_flashcards`: pending and dismissed drafts produce zero `Flashcard`s; only explicitly accepted drafts convert.
