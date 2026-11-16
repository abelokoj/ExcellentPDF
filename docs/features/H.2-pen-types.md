# Ph3: 39/49 | H.2: Pen types

**Tag:** H.2; **Priority:** P0; **Phase:** 3; **Needs:** H.1; **Code:** `excellent-ink::pens` (the presets),
`excellent-ink::raster` (the golden-image rasterizer), `excellent-canvas::page` (the `pen` prop), `excellent-app`
(`ink.pen_*` commands, the Write toolbar)
**Plan:** §8.4 and §27 entry `#### Ph3: 39/49 | H.2`.

## Problem

H.1 drew every stroke with one pen: one width curve, one amount of steadying, one solid line.

## User story

As a note-taker, I want to pick the pen I write with (a fountain pen for notes, a pencil for sketching, a marker for
headings) and have each one look and respond like itself.

## UI sketch

The Write mode's toolbar starts with six pens: Fountain, Ballpoint, Pencil, Brush, Calligraphy, Marker. The chosen
one shows pressed (filled with the primary colour); keys 1 to 6 on the pages pick them, and the palette lists them
under Ink.

## Mode

Write (the toolbar); the commands work in any mode.

## Speed and size impact

Nothing at startup. A pen is a small `Copy` value read once per stroke; a calligraphy nib adds one `atan2` and one
`sin` per sample.

## Design

- **Presets:** `PenKind::pen(size)` gives each pen its `WidthCurve` (minimum share, pressure exponent, tilt and
  speed), steadying, texture (`Smooth` or `Grain`), blend (`Normal`; `Multiply` is for H.3) and opacity:
  - Fountain: 35 % to full width on a 1.25 curve, thinned by speed.
  - Ballpoint: 80 % to full, linear, steadied more.
  - Pencil: half to full, widened by tilt, grain 0.45, opacity 0.85.
  - Brush: 5 % to full on a squared curve, thinned by speed, steadied most.
  - Calligraphy: a flat nib at 45° whose width falls to 15 % moving along it (`Nib::share`).
  - Marker: even and opaque.
- **Per stroke:** the canvas takes the pen from its `pen` prop (`{kind, size}`) when a stroke starts and keeps it
  for that stroke; `Pen::push` sets each point's width (with the nib's share), `Pen::finish` steadies and smooths it,
  and the `ink` event carries the pen's opacity.
- **Golden images:** `Raster::draw` draws a stroke's points as capsules with the pen's grain and opacity;
  `each_pen_matches_its_golden_image` replays the session fixture with every pen and compares with
  `tests/golden/<pen>.pgm`, a few levels of slack for platform rounding. `EXCELLENTPDF_UPDATE_GOLDEN=1` writes them
  again after a deliberate change.

## Acceptance tests

- [x] Each pen has a recorded-session golden image test: `each_pen_matches_its_golden_image` (and no two pens look
  alike).
- [x] The presets: `every_kind_has_its_own_name`, `a_nib_is_broad_across_and_thin_along`,
  `calligraphy_changes_width_with_direction`, `a_marker_is_even_and_a_brush_follows_the_hand`.
- [x] The rasterizer: `a_line_covers_its_band_and_nothing_else`, `grain_and_opacity_lighten_a_pencil`,
  `pgm_goes_out_and_back`.
- [x] The toolbar: `read_and_present_show_only_their_own_tools` (Write has the pens).

## Open

- The session behind the golden images is H.1's synthetic one (DD24); redo them from a recorded session.
- Texture and blend show in the golden images only: the saved annotation and the live preview are plain lines until
  H.9's appearance streams (DD25).
- The pen is chosen per pane and not kept across runs, in one colour and one size, until H.7's favourites (DD25).
- Enhanced has no pen types or nib code to port (its `Canvas_UnitTest*` tests cover the laser pointer and tools).
