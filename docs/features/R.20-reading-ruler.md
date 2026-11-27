# Ph1: 28/60 | R.20: Reading ruler (reading bar)

**Tag:** R.20; **Priority:** P1; **Phase:** 1; **Needs:** K.8, K.13; **Code:** `ecclesia-features::read::ruler`,
`ecclesia-canvas-win::canvas::reading`, `ecclesia-app::reading`
**Plan:** §27 entry `Ph1: 28/60 | R.20`.

## Problem

Readers who lose their line (dyslexia, long lines, tired eyes) have nothing to hold their place.

## User story

As a reader who skips lines, I want a band on the line I am reading that I can move a line at a time, so that I keep
my place.

## UI sketch

Palette, group **Reading**: Reading ruler (no key, as in Enhanced), Reading ruler: dim the page around it, Next line
(`↓`), Previous line (`↑`). The arrows move the ruler while the pages have the keyboard; with the ruler off they
scroll a line. The band follows the pointer when it moves, snapping to text lines; on a page without text it is a
free band at the pointer.

## Mode

Every reading mode. Off by default.

## Speed and size impact

- Startup: nothing.
- It reads the text of the pages shown and one either side (K.13), on the workers; one rounded fill per frame.

## Design

Model (`read::ruler`): `Style { height, color, opacity, invert }` with Enhanced's defaults (48 px, `#ffe082` at
`0x66`), `band` (centred on the line, taller for a taller line), `step` and `entry` (MuPDF's reading order, skipping
blank lines and the other pieces of a split line). The canvas draws the band darkest-wins, so black text under it stays
black; inverted, it dims the page outside it. Settings: `reading_ruler_height`, `reading_ruler_color`,
`reading_ruler_opacity`, `reading_ruler_invert`; no settings UI yet (K.16).

## Acceptance tests

- [x] `arrow_keys_move_the_ruler_exactly_one_text_line_in_two_columns`.
- [x] `blank_lines_and_pieces_of_one_printed_line_are_one_step`, `a_page_without_text_has_no_entry`,
  `the_band_holds_its_line_and_is_centred_on_it`, `the_tint_on_white_paper_matches_the_colour_at_its_opacity`,
  `hand_edited_settings_are_brought_into_range`, `the_ruler_prop_falls_back_to_defaults_value_by_value`.
- [ ] Tried by hand on a two-column paper.
