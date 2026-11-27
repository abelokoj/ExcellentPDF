# Ph5: 8/45 | B.11: Import and export

**Tag:** B.11; **Priority:** P1; **Phase:** 5; **Needs:** B.1; **Code:** `ecclesia-features::outline_extra::io`
**Plan:** §27 entry `#### Ph5: 8/45 | B.11: Import and export`.

## Problem

Power users migrating or editing large book outlines want to export, edit in external tools or scripts, copy as Markdown, and re-import without losing structure.

## User story

As a reader organizing a large PDF outline, I want to export and re-import outlines in indented text, JSON, pdftk `BookmarkBegin`, and Markdown formats with exact round-trip fidelity.

## UI sketch

- Outline sidebar menu: **Import outline…** (`outline.import`), **Export outline…** (`outline.export`), and **Copy outline as Markdown** (`outline.copy_markdown`).

## Mode

All modes.

## Speed and size impact

- Startup: zero cost; pure string parsers/serializers using existing `serde_json`.
- Installer size: zero new crates.

## Design

- **Model (`ecclesia-features::outline_extra::io`):**
  - `export_indented_text` / `import_indented_text`
  - `export_json` / `import_json` (preserves bold/italic/color styles and open flags)
  - `export_pdftk` / `import_pdftk` (`BookmarkBegin`, `BookmarkTitle`, `BookmarkLevel`, 1-based `BookmarkPageNumber`)
  - `export_markdown` / `import_markdown` (`- [Title](#page=N)` nested bullet lists)

## Acceptance tests

- [x] `outline_extra::io::tests::json_round_trips_losslessly`: verifies full style, open-state, URI, and hierarchy JSON round-trip.
- [x] `outline_extra::io::tests::indented_pdftk_and_markdown_round_trip_structure_and_pages`: verifies exact round-trip across indented text, pdftk `BookmarkBegin`, and Markdown.
