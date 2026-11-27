# Ph3: 43/49 | H.7: Favourite pens bar and presets

**Tag:** H.7; **Priority:** P0; **Phase:** 3; **Needs:** H.2; **Code:** `ecclesia-features::ink::favorites` (the
bar), `ecclesia-app::annotate::favorites` (settings, the bar and its dialog), `ecclesia-canvas::page` (the `pen`
prop's colour and pressure)
**Plan:** §8.4 and §27 entry `#### Ph3: 43/49 | H.7`.

## Problem

H.2's pens came in one colour and one width each, chosen per pane and forgotten on quitting (DD25).

## User story

As a note-taker, I want my own pens on a bar (a fine blue fountain pen for notes, a thick red marker for
corrections, a soft pencil for sketches), one press away, the same in every window and on every machine I use.

## UI sketch

The Write mode's toolbar starts with the bar: a round dot per pen in its colour, as wide as the pen writes, the one in
hand pressed; then `+`, then a rule and the mode's own tools (the erasers, the lasso, undo and redo). Press a pen to
take it; drag it along the bar to move it; right-click it for Edit pen and Remove pen from favourites. `+` (Ctrl+Alt+P)
saves the pen in hand again as a new one and opens it. The edit dialog (Ctrl+Alt+K for the pen in hand) has the kind,
the palette's colours, the width and the pressure response (Light, Normal, Firm); each choice shows on the bar at
once, and Escape or Done closes it. The palette also has Export favourite pens… and Import favourite pens…. Keys 1 to
7 (H.2) take the first saved pen of that kind, saving one as it comes when there is none.

## Mode

Write (the bar); the commands work in any mode.

## Speed and size impact

Nothing at startup beyond reading one settings key. The bar is at most 12 small buttons.

## Design

- **Model:** a `Favorite` is a kind, a colour, a width (0.25 to 24 pt) and a `Response` that multiplies the kind's
  width-curve exponent (Light 0.6, Normal 1, Firm 1.6). `Favorites` holds up to 12 and the one in hand; removing
  keeps the last one, reordering keeps the pen in hand in hand.
- **Settings:** `[ink]` in `settings.toml`, `{chosen, pens: [{kind, color, size, response}]}`; the default bar (a
  blue fountain pen, a black ballpoint, a pencil, a highlighter) writes no key. Unknown kinds are skipped, wrong
  values fall back to the kind's own. Settings sync (Y.2) carries the bar with every other setting.
- **Files:** `{format: 1, app, pens}`; a newer format, a file with no pens list, or one with no known pen is refused
  with a reason.
- **Canvas:** the `pen` prop gains `color` and `pressure` (the exponent factor).

## Acceptance tests

- [x] The bar syncs through settings: `the_bar_goes_through_settings_unchanged` (through `settings.toml`'s TOML and
  back, after adding and reordering).
- [x] `export_and_import_give_the_same_pens`, `reordering_keeps_the_pen_in_hand`,
  `the_bar_keeps_one_pen_and_at_most_max`, `a_kind_command_takes_its_first_pen_or_saves_one`,
  `a_pen_takes_its_colour_and_response`, and the canvas's `the_pen_prop_takes_a_favourite_s_colour_and_pressure`.

## Open

- Y.2 (settings sync) itself is Phase 9; until then the bar travels by export and import.
- The H.2 pen-kind buttons left the default Write toolbar (the bar replaces them); toolbars a reader customized keep
  theirs.
- Colours come from the palette; NSH's full colour picker for a pen waits for a reader asking for it.
