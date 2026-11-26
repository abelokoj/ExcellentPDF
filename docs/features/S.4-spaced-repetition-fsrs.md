# Ph4: 30/44 | S.4: Spaced repetition (FSRS)

**Tag:** S.4; **Priority:** P0; **Phase:** 4; **Needs:** K.20; **Code:** `excellent-store::study`, `excellent-features::study::srs`
**Plan:** §27 entry `#### Ph4: 30/44 | S.4` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Flashcards and vocabulary need one modern, scientifically validated spaced-repetition scheduler rather than maintaining separate SM-2 and Leitner implementations, while preserving existing users' schedules imported from SumatraPDF Enhanced.

## User story

As a learner, I want one FSRS-5 spaced-repetition engine for both flashcards and vocabulary, with an auditable review log, on-demand parameter optimization, and seamless migration from my existing SumatraPDF Enhanced vocabulary schedule.

## UI sketch

- Transparent engine behind Study review (S.5) and Vocabulary practice (S.7); settings expose desired retention (`0.90` default), maximum interval, and an "Optimize parameters" action in Study settings / command palette (`study.optimize_fsrs`).
- Optimization runs on a background worker with progress and cancel support.

## Mode

Study mode and Home page daily review.

## Speed and size impact

- Startup: zero startup cost; pure Rust math runs on demand.
- Budgets touched (plan §13.1): none; a single FSRS review calculation takes < 1 microsecond.
- Installer size: pure Rust implementation of FSRS-5 (19 weights, zero external ML crate overhead).

## Design

- **Model** (`excellent-features::study::srs`): `FsrsScheduler`, `FsrsParameters` (official 19 FSRS-5 default weights, `FSRS_DECAY = -0.5`, `FSRS_FACTOR = 19/81`), `Rating` (`Again`/`Hard`/`Good`/`Easy`), `CardPhase`, `CardState`, `ReviewOutcome`, `IntervalPreview`.
- **Optimizer**: `evaluate_log_loss` and `optimize_parameters` tune the 19 weights against `OptimizationItem` review histories via bounded gradient descent minimizing binary cross-entropy loss.
- **Migration**: `EnhancedScheduling` and `migrate_enhanced_state` convert SumatraPDF Enhanced's Leitner/SM-2 state (`box`, `ease`, `repetitions`, `intervalDays`, `dueTime`, `lastReviewed`, `learned`) into FSRS-5 `CardState` while preserving exact due timestamps and review history counts.
- **Storage** (`excellent-store::study`): `study_srs` and append-only `study_review_log` tables.

## Acceptance tests

- [x] `study::srs::tests::fsrs5_matches_reference_test_vectors`: scheduling matches the FSRS-5 reference implementation on its canonical test vectors and `R(S, S) == 0.9` identity.
- [x] `study::srs::tests::rating_intervals_are_monotonic_and_same_day_uses_short_term_formula`: rating monotonicity and same-day short-term stability update match FSRS-5.
- [x] `study::srs::tests::optimizer_improves_log_loss_on_synthetic_review_logs`: on-demand optimizer reduces log loss over review logs.
- [x] `study::srs::tests::enhanced_scheduling_migration_preserves_due_dates_and_progress`: Enhanced Leitner/SM-2 schedules migrate cleanly with exact due timestamps.
