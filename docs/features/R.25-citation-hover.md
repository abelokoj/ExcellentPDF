# Ph3: 3/49 | R.25: Citation and equation hover

**Tag:** R.25; **Priority:** P0; **Phase:** 3; **Needs:** K.13, R.7; **Code:** `ecclesia-features::read::refhover`
(`cite`, `layout`, `resolve`), `ecclesia-canvas::page::canvas::refhover`
**Plan:** §27 entry `#### Ph3: 3/49 | R.25` and the Enhanced parity list. The label is the entry's place in its phase
(plan §27.0, Tag rules).

## Problem

Reading a paper means flipping to the references for every `[12]` or `(Smith et al., 2020)`, and back to an earlier
page for every "see (14)", then finding the way back. Most papers have no links for these, and even where they do,
following one loses the place.

## User story

As a researcher reading a paper, I want to rest the pointer on a citation or an equation reference and see the entry
or the equation in a small preview, so that I can keep reading without losing my place.

## UI sketch

- Rest the pointer on a link, or on a citation in plain text, for the delay (300 ms by default). A preview appears
  below the pointer (above it when there is no room): the target region rendered from the page, with a soft shadow, a
  hairline frame and rounded corners, in the page's colours (dark mode recolours it like the pages).
- What it shows: the one bibliography entry (with its continuation in the next column when the entry wraps), the
  labelled equation, or, when neither is found, a landscape strip of the page below the target.
- The preview stays while the pointer is over it, and for 250 ms after the pointer leaves the source, so the pointer
  can travel to it. Clicking it goes to the target. Scrolling or zooming takes it down.
- Settings, Links: "Preview citations and equations" (on by default) and "Preview delay" (0 to 2000 ms, step 50).
- Keyboard and screen readers: the preview is a pointer convenience. The same target is reachable by following the
  link or by search; the preview has no focus stop of its own.

## Mode

All modes, on by default.

## Speed and size impact

- Startup: nothing. The worker thread starts on the first hover.
- The debounce, text extraction, lookup and render run off the UI thread. Answers carry a generation number, so a late
  answer for a citation the pointer has left is dropped.
- Plain-text lookups and their misses are cached per document; page texts are cached (256 pages).
- No new dependency.

## Design

Model, `ecclesia-features::read::refhover`, pure Rust, ported from SumatraPDF Enhanced (`RefHover*.cpp`):

- `cite`: numeric citations (`[12]`, `[3-5]`, ranges and lists across line breaks) and author-year citations
  (parenthetical and narrative), and finding their entries by plain text.
- `layout`: the region to show on a target page: one entry, a labelled equation, or a landscape strip; captions,
  two-column pages, hanging indents, footers and diagonal watermarks handled as Enhanced does.
- `resolve`: `Lookups::find` (the entry of a citation without a link: numeric first, then author-year, walking the
  pages from the last back, cached), `dest_y_from_source` (where a link without a y lands), `shown` (the region,
  trying the next page when the target is at a page's foot) and `zoom` (fits the region in the preview, 0.4 to 3.0).

View, `ecclesia-canvas::page::canvas::refhover`: one worker thread per canvas debounces hovers, resolves and renders
the region through the render pool, and wakes the canvas; the canvas uploads the slices and draws the preview after
the minimap.

Settings: `ref_preview` (flag, default on) and `ref_preview_delay` (ms, default 300), sent to the canvas as the
`hoverDelay` prop. No command.

## Acceptance tests

- [x] The tests ported from `RefHover_ut.cpp` in `refhover::tests` pass.
- [x] `hover_numeric_without_link_shows_entry`: hovering `[12]` in a paper without links finds reference 12 on the
  references page and shows that entry alone; a second hover comes from the cache.
- [x] `hover_misses_unlisted_number`
- [x] `dest_y_prefers_a_match_that_starts_its_line`
- [x] `popover_zoom_fits_room`
- [ ] Tried by hand on real papers, with and without links, in light and dark mode.

## Open

- Not tried by hand.
- No wheel zoom inside the preview (Enhanced has it).
- Enhanced's link-zoom path (a destination with its own zoom) is left out: our link targets carry no zoom.
- The page text cache is dropped whole when the document changes, not per page revision.
- The preview is drawn inside the canvas, so near the canvas edge it is clamped rather than overflowing the window.
