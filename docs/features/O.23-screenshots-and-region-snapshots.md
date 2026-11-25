# Ph3: 47/49 | O.23: Screenshots and region snapshots

**Tag:** O.23; **Priority:** P1 (parity); **Phase:** 3; **Needs:** K.8; **Code:** `excellent-features::snapshot` (the
region, its resolution and rendering), `excellent-canvas::page` (snapshot mode, the `snapshot` event),
`excellent-app::snapshot` (the command and dialog)
**Plan:** §27 entry `#### Ph3: 47/49 | O.23`.

## Problem

Taking a figure out of a PDF meant a screen grab at screen resolution: blurry when printed or pasted into slides.

## User story

As a student writing a report, I want to drag a box round a figure and paste it into my document, sharp at the size
I print it.

## UI sketch

Snapshot on the Read toolbar (Alt+S, the palette): the next drag on a page marks a region, as the marquee does. A
dialog opens: "Snapshot of page N", the resolution (96, 150, 300, 600 dpi; 300 first), the picture's size in pixels,
then Cancel, Save PNG… and Copy.

## Mode

Every mode; any document the pages show, editable or not.

## Speed and size impact

Nothing until used. The render runs on a worker: a page-wide region at 600 dpi is about 20 megapixels.

## Design

- **Render, not grab:** the region is drawn from the page's display list at `dpi / 72` times its size, on white
  (`render_area`), so text and vector figures are rendered afresh at that resolution.
- **A cap:** at most 40 megapixels; a larger region at a high resolution is rendered at the highest resolution that
  fits.
- **Out:** Copy puts the PNG on the clipboard as an image (`data:` URI, NSH's `set_clipboard_image`); Save PNG… asks
  where, named "<document> pN snapshot.png", and writes it atomically.
- **Enhanced:** its screenshots (`src/Screenshot*.cpp`) grab the screen with a global hotkey; the idea kept is the
  quick command and a file name ready to save.

## Acceptance tests

- [x] A 600 DPI snapshot of a vector figure is sharp: `a_600_dpi_snapshot_of_vector_text_is_sharp` (a region of
  vector text: the share of its ink that is anti-aliased edge grey falls well below the 72 dpi render's, at 3000 by
  1500 pixels for 360 by 180 points, and it saves as PNG).
- [x] `a_region_reads_either_way_round_and_needs_an_area`, `a_huge_region_is_rendered_as_large_as_fits`.

## Open

- "Make a card" (S.1) waits for study cards in Phase 4.
- The snapshot is of the page as drawn, annotations included; a choice to leave them out is not built.
