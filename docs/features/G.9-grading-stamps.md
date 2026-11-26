# Ph7: 6/32 | G.9: Grading stamps

**Tag:** G.9; **Priority:** P1; **Phase:** 7; **Needs:** A.20; **Code:** `excellent-features::grading` (`GradingStampKind`, `create_grading_stamp`, `stamp_points`)
**Plan:** §27 entry `#### Ph7: 6/32 | G.9`.

## Problem

Teachers grading student submissions need one-click pass/partial/error/bonus stamps carrying numeric point deltas that automatically feed score totals.

## User story

As a teacher grading a student PDF, I want customizable point-valued stamps (checkmark, cross, half-point, bonus, custom) whose point values sum cleanly into the grading rubric.

## UI sketch

- Floating grading stamp palette (`+1`, `-1`, `+0.5`, custom points) placing crisp stamp annotations directly on the submission page.

## Mode

Annotate and Grading modes.

## Speed and size impact

- Stamp metadata stores exact point deltas inside standard PDF annotation fields with zero external assets.

## Design

- **`GradingStampKind`**: Supports `Checkmark`, `Cross`, `Half`, `Bonus`, and `Custom` stamps paired with customizable point banks.
- **`create_grading_stamp` / `stamp_points`**: Produces standard `excellent_core::annot::Annotation` stamps encoding point deltas cleanly for live score aggregation (`recalculate_scores`) and PDF round-trip persistence.

## Acceptance tests

- [x] `grading_stamps_rubric_recalculation_and_comment_bank_round_trip`: places point stamps, inspects encoded `stamp_points`, and verifies live total recalculation.
