# Ph7: 9/32 | G.12: Batch open and export

**Tag:** G.12; **Priority:** P1; **Phase:** 7; **Needs:** G.10, F.6; **Code:** `ecclesia-features::grading` (`SubmissionBatch`, `SubmissionEntry`, `BatchExportSummary`)
**Plan:** §27 entry `#### Ph7: 9/32 | G.12`.

## Problem

Grading an entire class roster of PDF submissions requires instant next/previous student navigation, progress tracking, graded PDF batch export, and a gradebook CSV matching rubric and stamp totals.

## User story

As a teacher grading a folder of student PDFs, I want to step sequentially through all submissions (`Ctrl+]` / `Ctrl+[`), mark grades, export all annotated PDFs, and download a formula-safe gradebook CSV.

## UI sketch

- Top submission switcher bar showing `Student N of Total`, graded/ungraded completion badge, Previous/Next buttons, and "Export Graded Batch + CSV".

## Mode

Grading mode batch workspace.

## Speed and size impact

- Stores per-submission metadata and active rubric summary cleanly; batch CSV formatting escapes spreadsheet formula prefixes (`=`, `+`, `-`, `@`).

## Design

- **`SubmissionBatch` / `SubmissionEntry`**: Tracks student roster entries, active cursor, `next_submission`/`previous_submission` navigation, and completion state (`graded_count`).
- **Batch export (`export_batch_csv`, `export_graded_pdfs`)**: Generates an LMS-ready CSV summarizing rubric scores, stamp totals, and remarks alongside annotated output PDFs.

## Acceptance tests

- [x] `batch_fifty_submissions_export_with_matching_csv_totals`: steps through 50 student submissions, exports graded PDFs and CSV, and confirms CSV row totals match each submission rubric summary.
