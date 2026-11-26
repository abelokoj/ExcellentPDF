# Ph7: 32/32 | O.24: Color-accurate and print-production view

**Tag:** O.24; **Priority:** P2; **Phase:** 7; **Needs:** K.7; **Code:** `excellent-core::print_prod`
**Plan:** §27 entry `#### Ph7: 32/32 | O.24`.

## Problem

Prepress and print-production workflows require inspecting PDF/X `/OutputIntents` ICC profiles, inspecting CMYK and spot `/Separation` colorants, sampling total area ink coverage (TAC) against press limits, previewing trim/bleed boxes, and flagging hairline/overprint issues.

## User story

As a print designer or prepress reviewer, I want output-intent inspection, interactive CMYK/spot separation toggles, total ink coverage heatmaps/sampling, and automated print preflight checks.

## UI sketch

- Output Preview pane with `/OutputIntents` summary, `Cyan / Magenta / Yellow / Black / Spot` separation checkboxes, TAC limit threshold readout (`300%` warning highlight), and page Trim/Bleed overlay toggles.

## Mode

Print Production / Output Preview mode.

## Speed and size impact

- Separation preview and TAC sampling scan rendered RGBA/CMYK buffers directly with no heavy color-engine overhead.

## Design

- **Output intents & separations (`read_output_intents`, `inspect_separations`)**: Parses `/Catalog` `/OutputIntents` ICC profile metadata and enumerates `/Separation` / `/DeviceN` spot colorants plus `/OP`/`/op` overprint flags across pages.
- **CMYK & TAC analysis (`rgb_to_cmyk`, `sample_ink_coverage`, `summarize_pixmap_ink`, `apply_separation_preview`)**: Converts display samples to UCR/GCR CMYK + TAC%, flags pixels exceeding `max_tac_percent`, and filters rendered previews by active CMYK/spot channel visibility.
- **Preflight (`preflight_page`)**: Checks Trim/Bleed geometry, minimum hairline widths, overprint state, and ink coverage summaries.

## Acceptance tests

- [x] `ink_coverage_separations_and_preflight_checks_work`: reads `/OutputIntents`, detects `/Separation` colorants, samples CMYK/TAC coverage, filters separation previews, and exercises `preflight_page` findings.
