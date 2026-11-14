# K.6 Layout and coordinate transforms

**Tag:** K.6 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.3 · **Code:** `excellent-canvas::layout`,
`excellent-core::transform`, `excellent-mupdf::Document::page_box`, `excellent-canvas-win::placement`
**Plan:** §27 entry `#### K.6`.

## Problem

The canvas had one continuous layout and loaded every page on open to learn its size, so long documents opened slowly
and nothing could show facing, book, strip or grid views.

## User story

As a reader of long documents, I want any layout to appear at once, so that a 10,000-page file feels like a short one.

## UI sketch

None of its own: R.1–R.4 are its commands.

## Mode

Every mode.

## Speed and size impact

- Startup: nothing. On open, PDF page boxes are read from the page tree without loading pages.
- Layout of 10,000 pages: 0.19–0.49 ms by mode (measured, release). Budget: under 5 ms.
- Installer: none. `proptest` is a dev-dependency only.

## Design

- Model: `layout::Spec { mode, flow, direction }`; `Layout::new(sizes, spec)` gives page rectangles in bands, so
  `pages_in` and `page_at` are binary searches. `excellent-core::transform` maps page points to device pixels and back.
- View: `excellent-canvas-win::placement` places a page's points in the layout under the view rotation.

## Acceptance tests

- [x] `pages_never_overlap_and_stay_inside`, `bands_hold_every_page_once_in_order`, `queries_find_every_page`,
  `page_to_device_and_back`: property tests pass.
- [x] `lays_out_ten_thousand_pages_quickly` (ignored by default; run in release): 10,000 pages under 5 ms.
