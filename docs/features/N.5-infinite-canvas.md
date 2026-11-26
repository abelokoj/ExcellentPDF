# Ph4: 18/44 | N.5: Infinite canvas pages and whiteboards

**Tag:** N.5; **Priority:** P1; **Phase:** 4; **Needs:** N.4, H.1; **Code:** `excellent-features::notebook::infinite`
**Plan:** §27 entry `#### Ph4: 18/44 | N.5`.

## Problem

Brainstorming diagrams, mind maps, and long proofs overflow fixed paper boundaries.

## User story

As a designer or mathematician, I want an unbounded infinite canvas page where I can pan, zoom, and write freely anywhere without losing stroke precision, and export the result as one big PDF page or paper-sized tiles.

## UI sketch

- Insert > Infinite Whiteboard Page, pan/zoom camera, minimap overlay (R.17), Export dialog offering Single Page or Tiled (A4/Letter/etc.).

## Mode

Write mode (and Present mode via G.6).

## Speed and size impact

- Startup: zero cost.
- `f64` world coordinates prevent floating-point precision loss far from the origin; coordinates are shifted to local content origin in `f64` during PDF export.

## Design

- `InfiniteCanvas` stores `CanvasStroke`s (`CanvasPoint { x: f64, y: f64, width: f32 }`), `CanvasViewport` (`pan`, `zoom_at`, `screen_to_world`, `world_to_screen`), `minimap()` normalized projection, `content_bounds()`, and `export_pdf(InfiniteExportMode::SinglePage | Tiled)`.

## Acceptance tests

- [x] `drawing_far_from_origin_keeps_full_precision_and_exports_every_stroke`: draws strokes at `(10_000_000.0, -5_000_000.0)` with sub-point precision, verifies exact bounds, exports both SinglePage and Tiled Letter PDFs, and confirms MuPDF renders the strokes.
- [x] `viewport_pan_zoom_and_minimap_stay_consistent`: verifies coordinate round-trips and minimap bounds.
