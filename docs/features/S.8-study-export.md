# Ph4: 34/44 | S.8: Study export

**Tag:** S.8; **Priority:** P0 (parity); **Phase:** 4; **Needs:** A.14; **Code:** `excellent-features::study::export`
**Plan:** §27 entry `#### Ph4: 34/44 | S.8`, §11.7, and `sumatra_pdf_enhanced_plan.md` §5.15. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Students need to export highlights, notes and saved vocabulary with clickable page links into documents, spreadsheets or Markdown/Typst notes without losing metadata or introducing spreadsheet formula-injection risks.

## User story

As a student, I want to filter my annotations by page range, type, colour or selection and export them to Markdown, plain text, Typst, HTML, Word (`.docx`), JSON or CSV matching SumatraPDF Enhanced's output format.

## UI sketch

- Reachable via File / Study toolbar ("Export study notes"), shortcut and command palette (`study.export`).
- Dialog provides scope (document/page range or selection), page range, annotation type, colour, sort order (page order vs creation time), format picker (7 formats), live text preview and atomic Save button.
- Empty-filter state renders a clear notice; never overwrites the open source PDF.

## Mode

All modes (File menu, command palette) and Study mode toolbar.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched (plan §13.1): none at startup; large `.docx` exports run off the UI thread.
- Installer size: uses existing workspace `zip` crate for `.docx`; zero new dependencies.

## Design

- **Model** (`excellent-features::study::export`): `StudyFormat` (Markdown, Text, Typst, Html, Docx, Json, Csv), `StudyNoteKind`, `StudyNote`, `StudyDocument`, `StudyFilter`, `StudyOrder`, `select_notes`, `render_study`, `render_vocabulary_study`.
- **Enhanced parity helpers**: `study_date`, `study_timestamp`, `study_color`, `study_link`, `json_text`, `xml_text`, `typst_text`, `markdown_text`, `csv_text` (prefixes `'` on cells starting with `=`, `+`, `-`, `@`, `\t`, `\r`), `note_page_label`, `note_heading`, `note_details`, `note_body`, and OOXML `.docx` ZIP packaging.

## Acceptance tests

- [x] `study::export::tests::matches_enhanced_study_export_unit_tests`: output matches Enhanced's unit tests across all 7 formats (Markdown, Skim-style Text, Typst, HTML, DOCX, JSON, formula-safe CSV), filters and sort orders.
- [x] `study::export::tests::sticky_note_omits_redundant_note_prefix_and_vocab_exports_safely`: sticky notes omit redundant `Note:` headers and vocabulary CSV escapes formula-leading cells.
