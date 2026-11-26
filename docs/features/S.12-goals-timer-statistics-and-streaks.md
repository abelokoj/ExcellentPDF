# Ph4: 37/44 | S.12: Goals, timer, statistics and streaks

**Tag:** S.12; **Priority:** P1; **Phase:** 4; **Needs:** S.4; **Code:** `excellent-features::study::stats`, `excellent-store::study`
**Plan:** §27 entry `#### Ph4: 37/44 | S.12` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Sustained study benefits from clear daily goals, a distraction-free focus timer, retention charts and streak feedback, but caching derived statistics in duplicate columns causes drift whenever logs are imported or edited.

## User story

As a student, I want daily card/page/minute goals, a focus timer, retention and activity charts, and streaks computed directly from my review log and reading-time log so that numbers never drift out of sync.

## UI sketch

- Study statistics dashboard (Study mode tab and command palette `study.statistics`) displays daily goal rings, NSH `Plot` time-series charts over `daily_series`, retention percentage, focus timer controls (Start, Pause, Resume, Reset) and current/longest streak badge.
- Gentle streak celebration triggers only when `MotionPreference::Full` (respecting U.4 reduced motion).

## Mode

Study mode (and optional status-bar focus timer pill while running).

## Speed and size impact

- Startup: zero startup cost; logs are queried only when Study statistics or Home summary opens.
- Budgets touched (plan §13.1): none at startup; aggregation over thousands of log entries takes < 1 ms.
- Installer size: pure Rust model; zero new crates.

## Design

- **Model** (`excellent-features::study::stats`): `DailyActivity`, `GoalProgress`, `StreakInfo`, `StudyStatistics`, `MotionPreference`, `compute_statistics`, `FocusPhase`, `FocusTimer`.
- **Single source of truth**: `compute_statistics` derives totals, retention rate, daily plot buckets, today's goal progress and current/longest streaks exclusively from `excellent_store::study::ReviewLogRecord` and `ReadingLogRecord` slices — never double-stored.
- **Storage** (`excellent-store::study`): `study_goals`, `study_review_log`, `study_reading_log`.

## Acceptance tests

- [x] `study::stats::tests::statistics_and_streaks_are_computed_live_from_logs`: retention rate, daily series, goal progress, streaks and reduced-motion celebration gating are computed live from review and reading logs.
- [x] `study::stats::tests::focus_timer_transitions_through_work_pause_break_and_idle`: focus timer handles start, pause, resume, work->break session completion and reset.
