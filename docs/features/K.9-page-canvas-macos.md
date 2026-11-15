# Ph2: 1/3 | K.9: Page canvas on macOS

**Tag:** K.9; **Priority:** P0; **Phase:** 2; **Needs:** K.8; **Code:** `crates/excellent-canvas-mac`,
`excellent-canvas::page`
**Plan:** §27 entry `Ph2: 1/3 | K.9`; §5.4, §7.3; DD9, DD10.

## Problem

On macOS the reader has no page view: the `excellent.page-canvas` component exists only on Windows.

## User story

As a reader on a Mac, I want to open a PDF and scroll with the trackpad, pinch to zoom, select, follow links and write
with a tablet pen, as in any Mac document app.

## UI sketch

Nothing new: the same canvas, props, commands and events as K.8. Trackpad scrolling is smooth (precise deltas); a
pinch zooms about the fingers; Command with the wheel zooms; the pointer shows a hand over links and an I-beam over
text. At 100 % a page point is a screen point, as in Preview.

## Mode

Every mode; the canvas is the document view.

## Speed and size impact

- Startup: the view is made when the reader view is; tiles render on the shared workers.
- Budgets: frames draw with Core Graphics in `drawRect:` on a layer-backed view (DD10); 60 fps (120 on ProMotion) to
  be measured in K.24, with a `CAMetalLayer` host as the fallback.
- Size: the objc2 crates are NSH's own, at its versions.

## Design

- `host.rs`: `MacHost` implements `page::Host` with Core Graphics: tiles as `CGImage`s, `kCGBlendModeDarken` for
  darkest-wins highlights, even-odd fills for the reading aids, round strokes for ink. Workers wake the view on the
  main dispatch queue, by id.
- `view.rs`: `CanvasView` (an `NSView` subclass, flipped) owns the canvas, follows its size and backing scale, and
  takes `scrollWheel:`, `magnifyWithEvent:`, the mouse (tracking area for moves; `otherMouseDown:` for the middle,
  back and forward buttons) and tablet events (pressure; `tabletProximity:` for the eraser end).
- `component.rs`: the NSH component; `label` sets the accessibility label.
- The app registers it with `#[native_components(macos)]`.

## Acceptance tests

From **Done when**: Phase 1's reading checklist passes on macOS.

- [ ] macOS CI builds and tests the crate (`.github/workflows/macos.yml`).
- [ ] By hand on a Mac: the reading checklist.

## Open

The pen's tilt and coalesced tablet points; a Metal host if K.24 asks for one (DD10); trying it on a Mac.
