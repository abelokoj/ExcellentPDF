# Ph4: 35/44 | S.10: Exam mode

**Tag:** S.10; **Priority:** P1; **Phase:** 4; **Needs:** S.1, H.4; **Code:** `ecclesia-features::study::exam`
**Plan:** §27 entry `#### Ph4: 35/44 | S.10` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

When reviewing annotated study sheets before an exam, visible answers, margin notes and revealed definition tapes spoil self-testing, and manually toggling dozens of annotations back afterward is error-prone.

## User story

As a student preparing for a test, I want an exam mode that hides answers, notes and chosen annotation categories, covers H.4 tape strips, runs a timer, lets me reveal items one by one, and restores my exact prior visibility when I leave.

## UI sketch

- Triggered from Study mode toolbar ("Exam mode"), keyboard shortcut and command palette (`study.exam_mode`).
- Top exam bar shows remaining/elapsed timer, pause/resume, "Reveal next" button, remaining count and "Exit exam mode".
- Exiting immediately restores every hidden annotation, layer, notes/answers flag and H.4 tape opacity to its pre-exam state.

## Mode

Study mode (toolbar button visible by default).

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched (plan §13.1): none; entering and exiting exam mode compute pure diff sets in microseconds.
- Installer size: pure Rust model reusing H.4 `annotate::tape` and A.17 `annotate::categories`.

## Design

- **Model** (`ecclesia-features::study::exam`): `ExamConfig`, `VisibilitySnapshot`, `ExamVisibility`, `ExamTimerState`, `ExamSession`.
- **Lifecycle**: `ExamSession::enter(annotations, prior, config, now)` records exact prior `VisibilitySnapshot` (including every H.4 tape's opacity) and emits covering `Change`s; `reveal_one` / `reveal_next` reveal annotations or call `tape::tapped`; `pause_timer` / `resume_timer` / `remaining_secs` drive the timer; `exit(current_annotations)` restores the exact `VisibilitySnapshot` and returns `Change`s restoring every H.4 tape to its exact prior opacity.

## Acceptance tests

- [x] `study::exam::tests::exam_mode_hides_reveals_times_and_restores_exact_visibility`: hides notes/answers/categories, covers H.4 tape, tracks timer pause/expiry, reveals one at a time, and restores exact previous visibility and tape opacity on exit.
