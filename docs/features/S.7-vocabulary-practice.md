# Ph4: 33/44 | S.7: Vocabulary practice

**Tag:** S.7; **Priority:** P0 (parity); **Phase:** 4; **Needs:** S.4, S.6; **Code:** `ecclesia-features::vocabulary`, `ecclesia-store::study`
**Plan:** §27 entry `#### Ph4: 33/44 | S.7`, §11.7, and `sumatra_pdf_enhanced_plan.md` §5.12. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers who save new words while reading need structured practice activities and spaced repetition, and SumatraPDF Enhanced users must be able to bring their existing `SumatraPDF-vocabulary.json` library over without losing any words or due dates.

## User story

As a reader building my vocabulary, I want to save words with their reading context, practice across six interactive exercises scheduled by FSRS-5, and import my SumatraPDF Enhanced vocabulary file keeping every word and due date intact.

## UI sketch

- Accessible from the Home page learning hub, Study mode toolbar, dictionary lookup ("Add to vocabulary") and command palette (`vocabulary.open`).
- Shows the 11 named study decks, custom decks, Word of the Day, due review count, study-ahead toggle, search/filter and the 6 practice activities (Flashcards, Meaning Choice, Word Choice, Spelling, Word Scramble, Matching Pairs).
- Undo banner restores recently removed words; corrupt import leaves existing library untouched and reports a clear error.

## Mode

Home page learning hub and Study mode.

## Speed and size impact

- Startup: nothing loads at startup; vocabulary rows load lazily from SQLite when the learning hub or lookup opens.
- Budgets touched (plan §13.1): none at startup; Word of the Day is cached per UTC day and revision.
- Installer size: word-list packs load lazily/on demand; zero external crates added.

## Design

- **Model** (`ecclesia-features::vocabulary`): `VocabularyLibrary`, `VocabularyWord`, `VocabularyDeck`, `BUILTIN_DECKS` (Enhanced's 11 named study decks), `VocabActivity` (6 activities), `VocabularyQuestion`, `MatchPairsBoard`, `DueWindow` (`DueNow` / `StudyAhead`), `ImportMode` (`Merge` / `Replace`), `LearnedFilter`.
- **Scheduler**: every review runs through `crate::study::srs::FsrsScheduler` and emits a `ReviewLogRecord`.
- **Enhanced import/export**: `import_enhanced_json` parses both `sumatrapdf-vocabulary` v1 and `wmkeyboard-vocab` v1 (plain or gzip), migrating Leitner/SM-2 state via `srs::migrate_enhanced_state` while keeping every word, context, source page and exact `dueTime`.
- **Storage** (`ecclesia-store::study`): `save_to_store`, `load_from_store`, `record_review_in_store`.

## Acceptance tests

- [x] `vocabulary::tests::importing_enhanced_vocabulary_keeps_every_word_and_exact_due_dates`: importing an Enhanced vocabulary file preserves every word, deck, source page and exact due date and schedules subsequent reviews on FSRS-5.
- [x] `vocabulary::tests::all_six_practice_activities_undo_remove_and_sqlite_round_trip`: all 6 practice activities, undo removal, Word of the Day and SQLite round-trip pass.
