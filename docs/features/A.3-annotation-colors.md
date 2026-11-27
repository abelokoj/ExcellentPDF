# Ph3: 16/49 | A.3: Annotation colors

**Tag:** A.3; **Priority:** P0; **Phase:** 3; **Needs:** K.17; **Code:** `ecclesia-features::annotate::colors`,
`ecclesia-canvas::page::canvas::markbar`, `ecclesia-app::annotate::colors`
**Plan:** §27 entry `#### Ph3: 16/49 | A.3`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

One fixed row of colours for every markup makes a yellow underline or a black highlight easy and a reviewer's
colour code hard. Readers who can't tell red from green need colours they can tell apart.

## User story

As a reader, I want each markup tool to keep its own colour and opacity, choose them from a palette or any colour I
like, and switch to a colour-blind-safe palette, so that my markups mean what I want and I can read them.

## UI sketch

- The markup bar: each tool drawn in its colour; a short bar under the current tool (the last one pressed); then
  that tool's five swatches and a custom cell ("+" until it has a custom colour). A swatch recolours the tool and
  applies it to the selection.
- The colour dialog: a tool selector (Highlight, Underline, Squiggly, Strikeout, Proposal); the saturation and
  brightness area; hue and opacity sliders; the hex field and the opacity in per cent; the palette's swatches and the
  recent custom colours; Cancel and Done.
- Settings, Annotation: Palette (Standard, Colour-blind safe); Tool colours (Choose, opens the dialog).
- The command `annotate.colors` is in the palette and Annotate mode's toolbar; its page key is `Shift+C`.

## Mode

All modes; the toolbar button is in Annotate mode's toolbar.

## Speed and size impact

Nothing at startup: the colours are read from the settings when the canvas asks for its prop. The dialog is built
when opened.

## Design

- **Model** (`annotate::colors`, pure): `Tool`, `Palette` (standard, colour-blind safe) and its swatches per tool,
  `ToolStyle` (colour, opacity clamped to 0.1 to 1) and `Colors` (the palette, the tools given a style, six recent
  custom colours) with its JSON form.
- **Canvas** (`markbar`): reads `annotationColors`; a markup or proposal carries its tool's colour and opacity; a
  swatch sends `toolColor`, the custom cell `pickColor`.
- **App** (`annotate::colors`): the colours are kept in `settings.toml` under `[annotation]` (none when they are the
  defaults); the dialog edits a copy and keeps it on Done; the settings window's Annotation section sets the
  palette.

## Acceptance tests

- [x] `the_color_blind_safe_set_is_okabe_and_ito_s` (the colour-blind-safe set, U.4).
- [x] `every_palette_reads_on_paper` (black text reads at 4.5 : 1 on each highlighter colour; each line colour reaches 3 : 1 on white).
- [x] `a_tool_starts_from_its_palette_and_keeps_what_it_is_given`, `custom_colors_are_remembered_newest_first`.
- [x] `opacity_and_hex_stay_in_range`, `colors_go_through_json_and_back`.
- [x] `colors_are_kept_in_the_settings_and_read_back`, `the_picker_s_alpha_is_the_opacity`.
- [x] `the_theme_s_annotation_colors_are_the_highlighter_s` (§10.3's colours are the highlighter's).

## Open

- Not tried by hand.
- K.17 (themes) is in progress.
- The command opens the dialog on the highlighter; the bar's custom cell opens it on the bar's current tool.
