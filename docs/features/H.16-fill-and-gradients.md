# Ph4: 9/44 | H.16: Fill and gradients

**Tag:** H.16; **Priority:** P1; **Phase:** 4; **Needs:** H.6; **Code:** `ecclesia-ink::fill`
**Plan:** §8.4 and §27 entry `#### Ph4: 9/44 | H.16`.

## Problem

Closed handwritten shapes and diagrams can only be drawn as outlines; filling them with solid tints or smooth gradients requires exporting to an external vector editor.

## User story

As someone drawing diagrams and shaded figures in my notes, I want to fill a closed stroke or shape with a solid color or a linear or radial gradient saved directly in the PDF appearance stream so it renders identically in Acrobat and every other PDF viewer.

## UI sketch

Available in the lasso selection toolbar and annotation inspector ("Fill: Solid / Linear gradient / Radial gradient"), Write menu, and command palette (`ink.fill`, shortcut `Alt+F` with a closed stroke selected). Includes solid color swatch, gradient stop editor, angle/radius handle, and opacity slider; all controls keyboard-accessible.

## Mode

Write and Annotate.

## Speed and size impact

- Startup: nothing runs at startup; pure appearance-stream generator runs only when a fill is applied.
- Budgets touched: appearance stream generation takes < 0.1 ms per shape.
- Installer size: negligible (~6 KB compiled Rust, zero dependencies in `ecclesia-ink`).

## Design

- **Model (`ecclesia-ink::fill`):**
  - `Fill::Solid(SolidFill)`, `Fill::Linear(LinearGradient)`, and `Fill::Radial(RadialGradient)` with `ColorStop`, `ExtendMode`, `FillRule` (`NonZero` / `EvenOdd`), and optional `BorderStroke`.
  - `build_appearance` emits an ISO 32000-1 Form XObject (`AppearanceStream`) in standard PDF user space (y-up):
    - Solid fills use `rg`, path construction (`m`/`l`/`h`), and `f`/`f*`, with `/ExtGState /ca` for opacity < 1.0.
    - Linear gradients emit `/ShadingType 2` (Axial) and radial gradients emit `/ShadingType 3` (Radial), clipped to the closed path via `W n /Sh0 sh` (or `W* n /Sh0 sh`).
    - Two-stop gradients emit a PDF Type 2 exponential interpolation function; 3+ stop gradients emit a PDF Type 3 stitching function combining Type 2 sub-functions for universal Acrobat compatibility.
  - `sample_fill` provides CPU evaluation of solid, linear, and radial fills for live previews.
  - `is_closed_stroke` detects freehand strokes whose endpoints are close enough to fill.
- **Command id (for app wiring):** `ink.fill` (`Alt+F` with selection).
- **Storage:** written into the annotation's standard PDF `/AP /N` Form XObject appearance stream.

## Acceptance tests

- [x] `solid_fill_emits_standard_pdf_operators_and_extgstate`: verifies solid `rg`, path operators, `/BBox`, `/ExtGState /ca`, and Form XObject output.
- [x] `linear_gradient_emits_type2_shading_and_stitching_for_multi_stop`: verifies `/ShadingType 2`, Type 3 stitching function for 3 stops, `W* n /Sh0 sh`, and CPU sampling.
- [x] `radial_gradient_emits_type3_shading_and_samples_concentrically`: verifies `/ShadingType 3`, `/Coords`, Type 2 function, and radial sampling.
- [x] `h16_linear_and_radial_gradient_appearance_streams_render_in_mupdf` (in `ecclesia-features::ink::eyedropper::tests`): embeds generated linear and radial Form XObjects into real PDFs, renders them with MuPDF, and asserts the rendered pixel colors match the red→blue linear ramp and white→green radial ramp.
- [x] `validation_rejects_broken_inputs_and_detects_closed_strokes`: validates error handling and closed-stroke detection.
