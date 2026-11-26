# Ph7: 12/32 | F.16: Table and figure extraction

**Tag:** F.16; **Priority:** P2; **Phase:** 7; **Needs:** K.13; **Code:** `excellent-features::extract`
**Plan:** §27 entry `#### Ph7: 12/32 | F.16`.

## Problem

Extracting structured tables and figures from scientific papers or reports by hand loses column alignment, merges multi-line cells poorly, and risks spreadsheet CSV formula injection.

## User story

As a researcher reading a PDF, I want to detect tables and figures with nearby captions and copy or export tables to formula-safe CSV or Markdown and figures to high-DPI PNG or SVG.

## UI sketch

- Hover bounding boxes over detected tables (`Copy CSV`, `Copy Markdown`) and figures (`Save PNG`, `Save SVG`) with detected `"Table N:"` / `"Figure N:"` captions.

## Mode

Read and Research modes.

## Speed and size impact

- Reuses extracted `TextLayout` lines/spans without extra passes; PNG/SVG rendering crops directly at requested DPI via MuPDF.

## Design

- **Table extraction (`extract_tables`, `table_to_csv`, `table_to_markdown`)**: Clusters `TextLayout` lines into Y-rows and X-column gaps, merges multi-row headers/cells, attaches nearby captions (`find_nearby_caption`), and escapes CSV cells against formula prefixes (`=`, `+`, `-`, `@`, `\t`, `\r`).
- **Figure extraction (`extract_figures`, `render_figure_png`, `figure_to_svg_with_png_embed`)**: Detects graphical regions and captions (`Figure N` / `Fig. N`) and exports high-DPI PNGs or SVG wrappers with embedded base64 PNG data.

## Acceptance tests

- [x] `extracts_table_rows_columns_caption_csv_and_markdown`: builds a tabular PDF with nearby caption, verifies extracted rows, column count, caption matching, formula-safe CSV output, and Markdown formatting.
