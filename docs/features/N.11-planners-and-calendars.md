# Ph5: 20/45 | N.11: Planners and calendars

**Tag:** N.11; **Priority:** P2; **Phase:** 5; **Needs:** N.12, N.9; **Code:** `excellent-features::notebook::planners`
**Plan:** §27 entry `#### Ph5: 20/45 | N.11: Planners and calendars`.

## Problem

Students and knowledge workers want hyperlinked yearly/monthly/weekly/daily digital planners inside `.expdf` notebooks that also display their assignment deadlines and work in any PDF reader.

## User story

As a student, I want to generate a hyperlinked planner for any year with my due dates shown on day pages so every month, week, and day link works here and when exported as plain PDF.

## UI sketch

- File > New Notebook > **Yearly Planner…** (`notebook.new_planner`).
- Dialog picks year, week start (`Monday` / `Sunday`), page size, daily pages (`IncludeAllDays` / `CompactWeeksOnly`), and optionally pulls due items from `S.14`.

## Mode

All modes; opens in Write mode.

## Speed and size impact

- Startup: zero cost; generates PDF pages, `/Outlines`, and `/Link` annotations on demand via `PdfBuilder`.
- Compatibility: standard PDF 1.7 internal `/Link` annotations work in Acrobat, Preview, pdf.js, and MuPDF.

## Design

- **Model (`excellent-features::notebook::planners`):**
  - Gregorian leap-year calendar math (`CalendarDate`, `is_leap_year`, `days_in_month`).
  - `generate_planner` builds Year -> Month -> Week -> Day `.expdf` notebooks with bidirectional `/Link` annotations, `/Outlines` bookmarks, and `S.14` `PlannerDueItem` listings on day pages.

## Acceptance tests

- [x] `notebook::planners::tests::leap_year_planner_generates_valid_hyperlinks_and_due_dates`: verifies a 2024 leap-year planner builds 366 daily pages, every PDF `/Link` resolves in MuPDF, and `S.14` due items appear on the target day page.
