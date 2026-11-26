# Ph5: 24/45 | S.13: Summaries from your highlights

**Tag:** S.13; **Priority:** P1; **Phase:** 5; **Needs:** A.14; **Code:** `excellent-features::study::summaries`
**Plan:** §27 entry `#### Ph5: 24/45 | S.13: Summaries from your highlights`.

## Problem

Reviewing a densely highlighted textbook or paper requires jumping across dozens of pages instead of reading a structured outline of your own highlights and notes.

## User story

As a student, I want to generate a Markdown summary from my highlights and notes grouped under document headings completely offline, with an optional AI condensation mode that cites each highlight.

## UI sketch

- Study menu / Annotations tab > **Summarize Highlights** (`study.summarize_highlights`).
- Opens Markdown preview with copy/export and optional "Condense with AI" button (`SummaryCondenser`, disabled unless consented).

## Mode

Study, Annotate.

## Speed and size impact

- Startup: zero cost.
- Offline guarantee: `build_offline_summary` runs 100 % offline with zero packs or network access.

## Design

- **Model (`excellent-features::study::summaries`):**
  - `build_offline_summary` sorts `HighlightItem`s in `(page, y_pt, x_pt)` reading order, groups them under the nearest preceding `HeadingAnchor`, and emits clean Markdown.
  - `SummaryCondenser` trait defines the optional AI (`I.4`) condensation hook.

## Acceptance tests

- [x] `study::summaries::tests::offline_summary_groups_highlights_by_heading_in_reading_order`: verifies offline grouping by heading in reading order with page citations and notes.
