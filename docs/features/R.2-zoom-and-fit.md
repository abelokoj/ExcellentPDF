# R.2 Zoom and fit modes

**Tag:** R.2 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.6, K.7 · **Code:** `excellent-canvas::viewport`
**Plan:** §27 entry `#### R.2`.

## Problem

Fit width happened once on open; resizing the window lost it, and there was no fit page or fit height.

## User story

As a reader, I want the page to keep fitting the window as I resize it, so that I never re-zoom by hand.

## UI sketch

Palette, group **Zoom**: Zoom in (Ctrl+=), Zoom out (Ctrl+−), Actual size (Ctrl+1), Fit width (Ctrl+2), Fit page
(Ctrl+0), Fit height. Ctrl+wheel zooms about the pointer.

## Mode

Every mode.

## Speed and size impact

None measurable.

## Design

`ZoomMode` is kept by the viewport and re-applied on resize and relayout; zooming by hand switches to custom.

## Acceptance tests

- [x] `zooming_keeps_the_point_under_the_pointer` (proptest).
- [x] `fit_modes_stay_on_through_a_resize`.
- [ ] Fit content, presets, custom zoom dialog, pinch, per-document zoom: open.
