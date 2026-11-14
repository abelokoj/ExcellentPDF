# R.3 Reading direction: right-to-left and manga

**Tag:** R.3 · **Priority:** P0 · **Phase:** 1 · **Needs:** R.1 · **Code:** `excellent-canvas::layout`,
`excellent-mupdf::Document::reads_right_to_left`
**Plan:** §27 entry `#### R.3`.

## Problem

Manga and right-to-left books showed their spreads backwards.

## User story

As a manga reader, I want a right-to-left file to open in its order, so that spreads read correctly without settings.

## UI sketch

Palette, group **Layout**: Toggle right-to-left reading (manga). No default key (Enhanced has none).

## Mode

Every mode.

## Speed and size impact

One catalog lookup on open.

## Design

`Direction` in the layout's `Spec` mirrors rows and the strip. The PDF's `/ViewerPreferences /Direction` sets it on
open.

## Acceptance tests

- [x] `right_to_left_mirrors_the_rows_and_the_strip`, `page_boxes_are_read_without_loading_and_match_the_page`: a
  right-to-left PDF opens in the correct order without a setting.
- [ ] Arrow keys and swipe follow the direction: open.
