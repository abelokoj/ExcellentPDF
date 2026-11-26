# Ph7: 2/32 | R.26: CAD drawing view

**Tag:** R.26; **Priority:** P1; **Phase:** 7; **Needs:** R.14; **Code:** `excellent-core::cad`
**Plan:** §27 entry `#### Ph7: 2/32 | R.26`.

## Problem

Architectural, mechanical, and civil CAD exports in PDF often contain hairline monochrome vectors and AutoCAD/Revit OCG layers that lose contrast on dark backgrounds or become illegible at low zoom levels.

## User story

As an engineer reviewing CAD PDFs, I want automatic CAD detection, blueprint/dark-background line recoloring, zoom-aware hairline reinforcement, and CAD OCG layer inspection.

## UI sketch

- Page toolbar offers CAD view enhancement (`Light` / `DarkBlueprint`) when a CAD drawing is detected.
- Layers panel exposes CAD Optional Content Groups (`cad_layers`, `layer_visibility_map`) with per-layer visibility state.

## Mode

Read and Annotate modes, enabled automatically or manually when viewing vector drawings.

## Speed and size impact

- Startup: zero startup cost.
- Runtime: detection scores PDF/E markers, Producer/Creator metadata, and page stream geometry in linear time; pixmap transform scans rendered pixels in place (`enhance_cad_pixmap`).

## Design

- **Detection (`detect_cad_pdf`)**: Evaluates `ISO_PDFEVersion`, blacklisted non-CAD producers, CAD producer/creator strings, vector stroke-to-text/fill ratios, page dimensions, OCG counts, and `/Square` markup hints (`CadDetection`, `CadPolicy`).
- **Color mapping & hairline rules (`cad_map_color`, `cad_min_line_width_for_zoom`, `enhance_cad_pixmap`)**: Maps neutral CAD grayscales cleanly onto dark-blueprint or high-contrast light palettes while preserving semantic color lines and computing minimum stroke width for crisp rendering at any zoom.
- **Layer metadata (`cad_layers`, `layer_visibility_map`)**: Extracts MuPDF OCG layers into deterministic name-to-visibility maps.

## Acceptance tests

- [x] `detects_cad_drawings_and_rejects_non_cad_corpus_files`: detects synthetic CAD/PDF-E drawings with OCG layers and hairlines while rejecting non-CAD corpus documents (`zlib-manual.pdf`, `base14-outline-annot.pdf`, prose PDFs).
- [x] `cad_color_map_and_dark_blueprint_pixmap_transform`: verifies dark-blueprint color mapping, hairline width floor, and pixel transformation.
