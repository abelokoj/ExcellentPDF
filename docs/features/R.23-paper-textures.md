# Ph4: 2/44 | R.23: Paper textures

**Tag:** R.23; **Priority:** P2; **Phase:** 4; **Needs:** R.15; **Code:** `ecclesia-core::darkmode::paper` (the grain),
`ecclesia-canvas::page` (drawn over each page, the `paperTexture` prop; `Host::faint_image` on Windows, macOS and
GTK), `ecclesia-app` (the setting)
**Plan:** §27 entry `#### Ph4: 2/44 | R.23`.

## Problem

On a bright screen a page is a flat white rectangle; some readers find a page that looks like paper calmer to read.

## User story

As someone reading for hours, I want the page to look like paper, without the document itself changing.

## UI sketch

Settings, Reading: "Paper texture" (off by default). On, a faint grain lies over every page; with a reading theme it
tints that theme's paper.

## Mode

Every mode.

## Speed and size impact

Off: nothing. On: one 512 by 512 grain picture made once (on the first frame after it is turned on, and again after
the graphics device is lost) and drawn once per visible page per frame, stretched over the page.

## Design

- **The grain:** white noise softened by one 3 by 3 box blur that wraps at the edges (so it tiles without a seam),
  shaped to stay light (about 200 to 255): soft fibres, never dark. The same on every run.
- **Never in the tiles:** the canvas lays it over each page after the page's tiles, at 6 % opacity, through a new
  host call that draws a picture at an opacity (`Host::faint_image`: Direct2D's bitmap opacity, Core Graphics'
  alpha, a GTK opacity node). Tiles, search, copy, print and export never see it.
- **With the page:** one picture stretched over the page, so the grain scrolls, zooms and turns with the paper.

## Acceptance tests

- [ ] No measurable effect on scroll frame time: not measured here (the budget script measures cold start and
  opening only); the maintainer's to measure with the setting on and off.
- [x] `the_grain_is_light_and_varied_and_the_same_every_time`, `the_grain_tiles_without_a_seam`.

## Open

- The frame-time measurement above.
- One grain for every theme; a choice of textures (laid paper, recycled) is not built.
