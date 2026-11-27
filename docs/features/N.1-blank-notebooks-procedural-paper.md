# Ph4: 14/44 | N.1: Blank notebooks with procedural paper

**Tag:** N.1; **Priority:** P0; **Phase:** 4; **Needs:** N.4, K.8; **Code:** `ecclesia-features::notebook::paper`
**Plan:** §27 entry `#### Ph4: 14/44 | N.1`.

## Problem

Users creating study notes or lecture notebooks need crisp lined, grid, dot, Cornell, music, graph, isometric, and planner stationery without bloating file sizes with raster background images.

## User story

As a student or researcher, I want to start a blank notebook with procedural stationery in any standard or custom page size and reading theme, so that my pages stay tiny and export identically to PDF.

## UI sketch

- Entry points: File > New Notebook, toolbar button, command palette (`notebook.new`), shortcut (`Ctrl+Shift+N`).
- Picker shows the 10 templates (blank, lined, college-ruled, grid, dot, Cornell, music, graph, isometric, planner), paper sizes (A4, Letter, A5, Phone, Custom), orientation, and theme paper tint.
- Accessible radio groups with keyboard navigation.

## Mode

Write mode (and Create flow from Home).

## Speed and size impact

- Startup: zero cost; procedural geometry is generated only when rendering a notebook page or exporting.
- File size: template parameters take bytes in `notebook.json` rather than megabytes of raster backgrounds.
- Installer: pure Rust vector math, zero extra dependencies.

## Design

Model (`ecclesia-features::notebook::paper`):
- `PaperSpec` holds spacing, margins, ruling/accent/paper colors, and stroke widths for all 10 `PaperTemplateId` variants.
- `PaperSpec::with_reading_theme` integrates with R.15 `ReadingTheme`.
- `PaperSpec::primitives(w, h)` emits `Vec<PaperPrimitive>` (`FillRect`, `Line`, `Dot`, `StrokeRect`) shared verbatim between live canvas underlay drawing and `to_pdf_content_stream` / `export_single_page_pdf`.

## Acceptance tests

- [x] `all_ten_procedural_templates_export_and_render_in_mupdf`: generates primitives and exports vector PDFs for all 10 templates, confirming identical coordinate geometry and visible MuPDF rendering.
- [x] `paper_spec_adapts_to_reading_themes_and_page_sizes`: verifies R.15 reading theme tinting and custom/landscape dimensions.
