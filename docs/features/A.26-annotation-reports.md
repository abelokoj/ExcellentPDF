# Ph5: 13/45 | A.26: Annotation reports

**Tag:** A.26; **Priority:** P2; **Phase:** 5; **Needs:** A.14; **Code:** `ecclesia-features::annotate::report`
**Plan:** §27 entry `#### Ph5: 13/45 | A.26`.

## Problem

Readers need shareable, filterable annotation summaries (including Markdown deep links and visual page crops) across Markdown, HTML, CSV, and PDF.

## User story

As a reviewer, I want to export all or filtered annotations as Markdown, HTML, CSV, or PDF summary with page numbers, quotes, notes, categories, deep links, and page crops, so that I can share or archive my review findings.

## UI sketch

- Annotations sidebar "Report…" action allowing format choice (`Markdown`, `HTML`, `CSV`, `PDF`) and respecting the active annotation filter (`A.14`/`A.19`).

## Mode

Read, Annotate, Study.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched: page crops render on demand via `DisplayList::render_area` at background priority.
- Installer size: 0 bytes added (uses `ecclesia_mupdf::html_to_pdf` and `base64`).

## Design

- Model: `ecclesia-features::annotate::report::{ReportFormat, ReportConfig, ReportItem, build_report_items, attach_page_crops, render_annotation_crop, export_markdown, export_html, export_csv, export_pdf_summary}`.

## Acceptance tests

- [x] `generates_markdown_html_csv_and_pdf_reports_with_deep_links`: Verifies Markdown deep links (`ecclesiapdf://open?...`), HTML with embedded PNG crop, formula-safe CSV, and valid compiled PDF summary.
