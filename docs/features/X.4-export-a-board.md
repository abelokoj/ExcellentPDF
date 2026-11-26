# Ph9: 8/13 | X.4: Export a board

**Tag:** X.4; **Priority:** P2; **Phase:** 9; **Needs:** X.1; **Code:** `excellent-features::canvas_board::export`
**Plan:** §27 entry `#### Ph9: 8/13 | X.4` and §11.10.

## Problem

Boards need to leave the app as portable documents, vector diagrams, raster summaries, outlines, or self-contained `.exboard` archives that open reliably in external tools.

## User story

As a researcher, I want to export a board as Markdown with `excellentpdf://` deep links, SVG, PNG summary, hierarchical outline, or a self-contained `.exboard` ZIP archive with embedded assets.

## UI sketch

- Export submenu in Canvas Board (`canvas.export.markdown`, `canvas.export.svg`, `canvas.export.png`, `canvas.export.outline`, `canvas.export.exboard`).

## Mode

Research, Study.

## Speed and size impact

- Startup: exporter runs only on explicit export invocation.
- Budgets touched: pure-Rust SVG/PNG/ZIP generation finishes in milliseconds.
- Installer size: 0 bytes added (uses `flate2` + `zip`).

## Design

- Model: `excellent-features::canvas_board::export::{export_board_markdown, export_board_outline, export_board_svg, export_board_png_summary, export_exboard_zip, import_exboard_zip}`.

## Acceptance tests

- [ ] `board_exports_svg_png_markdown_outline_and_exboard_zip_roundtrip`: Verifies valid SVG, PNG signature bytes, deep-linked Markdown, outline, and lossless `.exboard` ZIP round-trip with binary assets.
