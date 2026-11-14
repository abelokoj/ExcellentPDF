# R.1 Page layouts

**Tag:** R.1 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.6, K.8 · **Code:** `excellent-canvas-win::view`,
`excellent-app::commands`
**Plan:** §27 entry `#### R.1`.

## Problem

Only one continuous column of pages: no two-page spreads, no book view, no strip, no overview grid.

## User story

As a reader of books and magazines, I want spreads with the cover alone, so that pages pair the way they were printed.

## UI sketch

Command palette, group **Layout**: Single page (Ctrl+6), Facing pages (Ctrl+7), Book view (Ctrl+8), Horizontal strip,
Page grid, Toggle continuous scrolling. Toolbar toggle and menus are open (no toolbar yet).

## Mode

Every mode; continuous single page by default.

## Speed and size impact

- Switching relays out in well under a millisecond (K.6); tiles are kept.

## Design

`View { spec, rotation, page_box }` and `ViewCommand` → `Effect` (relayout, rerender, remeasure, fit). The viewport's
`anchor` and `relayout` keep the reading position. Kept per canvas until K.5.

## Acceptance tests

- [x] `a_new_layout_keeps_the_reading_position`: switching layouts keeps the same text at the top.
- [x] `book_view_puts_the_cover_alone_on_the_right`: book layout puts page 1 alone.
