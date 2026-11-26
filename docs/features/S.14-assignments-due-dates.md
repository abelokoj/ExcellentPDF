# Ph5: 25/45 | S.14: Assignments with due dates

**Tag:** S.14; **Priority:** P2; **Phase:** 5; **Needs:** F.6; **Code:** `excellent-features::study::assignments`
**Plan:** §27 entry `#### Ph5: 25/45 | S.14: Assignments with due dates`.

## Problem

Students reading textbooks and syllabi need due dates, reading page ranges, and checklists tied directly to their PDFs with reminders that survive app restarts.

## User story

As a student, I want to attach due dates and checklists to documents or page ranges so OS reminders fire on time even after restarting the app and show up in my calendar and planner.

## UI sketch

- Study panel > **Assignments** (`study.assignments`) lists upcoming deadlines, checklist progress, calendar view (`NSH Calendar`), and due reminders via K.19 notifications.

## Mode

Study.

## Speed and size impact

- Startup: lightweight query on `study_assignment` due index only when reminders/Study panel are active.
- Storage: SQLite (`study_assignment` + `study_assignment_check`) with `reminder_fired_at` deduplication.

## Design

- **Model (`excellent-features::study::assignments`):**
  - `AssignmentStore` manages CRUD, checklist item completion (`AssignmentStatus` auto-updates), `in_window` calendar queries, and `poll_due_reminders`.

## Acceptance tests

- [x] `study::assignments::tests::reminders_fire_at_exact_time_after_restart_without_duplicates`: verifies SQLite persistence across reopen (restart), exact reminder window firing, and no duplicate firing.
