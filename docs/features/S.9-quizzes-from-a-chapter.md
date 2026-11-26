# Ph6: 3/13 | S.9: Quizzes from a chapter

**Tag:** S.9; **Priority:** P1; **Phase:** 6; **Needs:** I.3, S.1; **Code:** `crates/excellent-features/src/ai/quiz.rs`
**Plan:** §27 entry `#### Ph6: 3/13 | S.9`.

## Problem

Self-testing after reading a textbook chapter requires writing practice questions manually or risking ungrounded AI trivia.

## User story

As a student, I want to generate multiple-choice and short-answer quiz questions from a chapter's page range where every question cites its source passage, edit/accept the ones I want, and run them in Exam Mode.

## UI sketch

- Study mode / Outline chapter context menu: **Generate Chapter Quiz**; opens a review sheet with inline edit, **Accept**, **Dismiss**, **Add to Flashcards**, and **Start Exam Quiz** actions.

## Mode

Study mode when AI is enabled.

## Speed and size impact

- Startup: none. Generates drafts asynchronously over the selected page range only.

## Design

- `generate_chapter_quiz` retrieves chunks strictly inside `page_range`, parses JSON-schema `MultipleChoice` and `ShortAnswer` drafts, and verifies every question's `Citation` against real passages in that page range via `verify_quote_in_chunk`.
- `ChapterQuizDraft::into_accepted_flashcards` exports ONLY accepted questions to `study::cards::Flashcard`.
- `QuizExamRunner` integrates directly with `study::exam::ExamSession` (hiding notes, answers, and tape on entry, scoring responses, and restoring exact visibility on exit).

## Acceptance tests

- [x] `chapter_quiz_verifies_passages_requires_acceptance_and_bridges_to_exam_and_cards`: out-of-range/unverified questions are dropped, unaccepted drafts never create flashcards, and accepted questions run through `ExamSession` and restore visibility cleanly.
