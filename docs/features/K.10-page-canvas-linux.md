# Ph2: 2/3 | K.10: Page canvas on Linux

**Tag:** K.10; **Priority:** P0; **Phase:** 2; **Needs:** K.8; **Code:** `crates/excellent-canvas-gtk`,
`excellent-canvas::page`
**Plan:** §27 entry `Ph2: 2/3 | K.10`; §5.4, §7.3; DD9.

## Problem

On Linux the reader has no page view: the `excellent.page-canvas` component exists only on Windows, so the window
opens with an empty area where the document belongs.

## User story

As a reader on GNOME or another Linux desktop, I want to open a PDF and scroll, zoom, select, follow links and write
with a pen as I can on Windows.

## UI sketch

Nothing new: the same canvas, the same props, commands and events as K.8, inside the same NSH window. The pointer
shows a hand over links, an I-beam over text and a grab hand while middle-click scrolling.

## Mode

Every mode; the canvas is the document view.

## Speed and size impact

- Startup: the widget is made when the reader view is; tiles render on the shared workers.
- Budgets: frames are GSK render nodes, drawn by GTK's GPU renderer; scroll and ink latency to be measured in K.24.
- Size: no new dependency; GTK comes through NSH's Linux backend.

## Design

- `host.rs`: `GtkHost` implements the shared `page::Host`. GTK draws only in a widget's `snapshot`, so the widget
  lends it for the frame and `frame_ready` is false between frames. Tiles are `GdkMemoryTexture`s (RGB); darkest-wins
  highlights blend the frame so far with `GskBlendMode::Darken`; workers wake the widget through the default main
  context.
- `widget.rs`: `CanvasWidget` (a `GtkWidget` subclass) owns the canvas, follows its size and the surface's scale
  (fractional on Wayland), and wires the wheel and touchpad (`EventControllerScroll`), the mouse
  (`EventControllerMotion`, `GestureDrag`, `GestureClick` for the middle, back and forward buttons) and the pen
  (`GestureStylus`: pressure, eraser end).
- `component.rs`: the NSH component; `label` sets the accessible name.
- The app registers it with `#[native_components(linux)]`.

## Acceptance tests

From **Done when**: Phase 1's reading checklist passes on GNOME (Wayland) and one X11 desktop.

- [ ] Linux CI builds and tests the crate (`.github/workflows/linux.yml`).
- [ ] By hand on GNOME (Wayland): the reading checklist.
- [ ] By hand on an X11 desktop: the reading checklist.

## Open

The pen's tilt and its coalesced samples (`GestureStylus::backlog`); touch gestures; trying it on Wayland and X11.
