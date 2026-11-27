# Ph7: 7/32 | G.10: Rubric panel and scores

**Tag:** G.10; **Priority:** P1; **Phase:** 7; **Needs:** G.9; **Code:** `ecclesia-features::grading` (`Rubric`, `recalculate_scores`, `create_final_score_stamp`)
**Plan:** §27 entry `#### Ph7: 7/32 | G.10`.

## Problem

Manual tallying across rubric criteria and inline point stamps is error-prone and slows down consistent grading and final score stamping.

## User story

As a teacher grading against a multi-criterion rubric, I want rubric scores and page stamp points tallied automatically, clamped or flagged cleanly, and stamped onto the cover page.

## UI sketch

- Side panel showing criteria (`RubricCriterion`), max points, earned points, auto-recalculated stamp adjustments, percentage grade, and a "Stamp final score" action.

## Mode

Grading mode sidebar.

## Speed and size impact

- Rubric recalculation is instant in-memory arithmetic (`O(criteria + stamps)`).

## Design

- **`Rubric` / `CriterionScore` / `RubricSummary`**: Stores weighted or raw criteria scores, aggregates `stamp_points` from annotations on the document, and computes earned/max totals and percentages via `recalculate_scores`.
- **`create_final_score_stamp`**: Generates a formatted cover/summary score stamp annotation (`N / M (P%)`).

## Acceptance tests

- [x] `grading_stamps_rubric_recalculation_and_comment_bank_round_trip`: validates multi-criterion scoring, live recalculation from grading stamps, and final score stamp creation.
