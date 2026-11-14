# R.4 Rotation and crop-box view

**Tag:** R.4 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.6 · **Code:** `excellent-core::transform`,
`excellent-core::view`, `excellent-canvas-win::placement`
**Plan:** §27 entry `#### R.4`.

## Problem

A sideways scan could not be turned upright for reading, and pages always showed their crop box.

## User story

As a reader of scanned documents, I want to turn the view a quarter turn, so that I can read a sideways page without
changing the file.

## UI sketch

Palette, group **Page**: Rotate view clockwise (Ctrl+Shift+=), Rotate view anticlockwise (Ctrl+Shift+−), Show media /
crop / bleed / trim / art box.

## Mode

Every mode. The rotation is view state, not saved to the file (O.1 rotates the file).

## Speed and size impact

Rotation re-renders the visible tiles; a box change re-reads the boxes on a background thread.

## Design

Tiles render with `Transform { zoom, rotation }`. Ink is stored per page in page points and drawn through `Placement`.

## Acceptance tests

- [x] `page_to_device_and_back`, `a_point_comes_back_under_every_rotation`: annotations stay correct under every
  rotation and box. Selection will use the same placement.
