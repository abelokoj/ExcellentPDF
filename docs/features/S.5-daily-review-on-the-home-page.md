# Ph4: 31/44 | S.5: Daily review on the home page

**Tag:** S.5; **Priority:** P0; **Phase:** 4; **Needs:** S.4, F.17; **Code:** `ecclesia-features::study::review`
**Plan:** §27 entry `#### Ph4: 31/44 | S.5` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Learners forget to open separate review panels unless due counts are surfaced directly on the home page and reviewing a large queue feels instantaneous without stuttering on PDF page renders.

## User story

As a student, I want to see how many cards are due on the home page and run a keyboard-driven (`1`–`4`) review session where every card's source crop appears immediately without waiting on rendering.

## UI sketch

- Home page displays a "Daily review" card showing `due_now`, `new_cards`, `overdue` and `reviewed_today` counts alongside a "Start review" button (also in Study toolbar and palette `study.start_review`).
- Review screen shows front/back, cached source crop thumbnail, "Go to source" link, Space to flip, and keys `1` (Again), `2` (Hard), `3` (Good), `4` (Easy) with live interval previews.
- Empty state congratulates the user when zero cards are due; all controls have keyboard focus and accessible names.

## Mode

Home page (visible by default when cards exist) and Study mode.

## Speed and size impact

- Startup: home page runs a single indexed `SELECT COUNT(*) FROM study_srs WHERE due <= ?1` query (< 1 ms).
- Budgets touched (plan §13.1): zero UI-thread stalls during review because `CropCache` pre-warms crops ahead of the cursor on background workers.
- Installer size: pure Rust model; zero new crates.

## Design

- **Model** (`ecclesia-features::study::review`): `DailySummary` + `daily_summary`, `CropKey`, `CropImage`, `CropCache` (bounded FIFO/LRU cache with `prewarm`), `rating_from_key('1'..='4')`, and `ReviewSession` (`from_due`, `prefetch_crops`, `current`, `current_crop`, `reveal_answer`, `interval_preview`, `grade`, `grade_key`).
- **Storage**: reads due cards from `ecclesia-store::study::due_cards` / `due_count` and persists each `ReviewOutcome` via `save_srs` + `append_review_log`.

## Acceptance tests

- [x] `study::review::tests::review_session_of_100_cards_uses_cached_crops_without_blocking`: a review session of 100 cards pre-warms its crop cache and serves every card's crop immediately without re-rendering or waiting.
