# Ph5: 26/45 | S.15: Language learning

**Tag:** S.15; **Priority:** P2; **Phase:** 5; **Needs:** S.6, S.7, U.5; **Code:** `ecclesia-features::study::language`
**Plan:** §27 entry `#### Ph5: 26/45 | S.15: Language learning`.

## Problem

Reading documents in a foreign language requires switching between dictionaries, pronunciation audio, translation tools, and flashcard apps.

## User story

As a language learner, I want to tap a word to see its dictionary definition, hear its pronunciation, add it to my flashcard deck in two taps, or view a paragraph-aligned parallel translation pane.

## UI sketch

- Word tap popover shows headword, IPA, definition, translation, **Pronounce** button (`U.5`), and **Add to Deck** button.
- View > **Parallel Translation** (`study.parallel_translation`) opens a split pane (`V.1`) synchronized by paragraph index.

## Mode

Study, Read.

## Speed and size impact

- Startup: zero cost.
- Storage: writes directly to `ecclesia-store::study` FSRS-5 tables (`study_deck`, `study_card`, `study_srs`).

## Design

- **Model (`ecclesia-features::study::language`):**
  - `inspect_tapped_word`, `pronounce_word`, `capture_word_to_deck` (single-transaction deck + FSRS card creation).
  - `ParallelTranslationPane` maintains 1-to-1 `AlignedParagraph` pairs with synchronized `focus_paragraph`.

## Acceptance tests

- [x] `study::language::tests::word_goes_from_page_to_deck_and_pronounces`: verifies tap lookup, TTS pronunciation, and FSRS SQLite deck capture.
- [x] `study::language::tests::parallel_pane_aligns_source_and_translated_paragraphs`: verifies 1-to-1 paragraph alignment and synchronized focus.
