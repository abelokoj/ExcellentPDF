# Ph3: 7/49 | V.8: Synchronized scrolling

**Tag:** V.8; **Priority:** P1; **Phase:** 3; **Needs:** V.1; **Code:** `excellent-features::split`,
`excellent-app::panes::sync`, the page canvas's `follow` command (`excellent-canvas::page`)
**Plan:** §27 entry `#### Ph3: 7/49 | V.8`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Comparing two versions of a paper, a translation beside its original, or slides beside their notes means scrolling
two panes by hand and keeping them level. Documents with different page sizes or front matter drift apart at once.

## User story

As a reader with two documents side by side, I want the other pane to follow the one I scroll, by page, by position
through the document, or from pages I matched myself, so that I read both at the same place.

## UI sketch

- Each split pane's header has a "Sync" menu button, in the primary colour while sync is on. Its items:
  "Synchronized scrolling" (Ctrl+Alt+S, checked while on); "Sync panes by page", "by position", "by page pairs"
  (one checked); "Sync zoom between panes" (checked while on); "Pair these pages"; "Clear page pairs" (dimmed with
  none).
- All of them are in the command palette, under Panes.
- Settings, Tabs: "Synchronized scrolling" (Same page; Same position; Page pairs) and "Sync zoom" (a switch).
- Off at every start. Turning it on lines the other panes up with the active one at once.

## Mode

All modes; only while the window is split.

## Speed and size impact

- Nothing runs while sync is off but a copy of each view report the canvas already sends. On, a pane's view report
  sends one `follow` command per other pane; following moves the view without a jump on the history.
- The canvas reports a view when a scroll or zoom settles, so followers catch up as each wheel step ends rather than
  frame by frame.
- No new dependency.

## Design

- The model (`excellent_features::split`) maps a place (page, across, down) from one pane's pages to another's:
  - Same page: the same page number and place on it, stopping at the last page.
  - Same position: the same share of the whole document's height, measured with each document's own page heights,
    so documents of different page sizes and lengths stay level.
  - Page pairs: the same number of pages on from the nearest pair at or before the page (else the first pair).
- "Pair these pages" records every open pane's page at once, switches to page pairs and turns sync on. A pair at
  the same page of the first pane replaces the old one.
- The canvas tags the view it reports after a `follow` (`"follow": true`): the app takes it as a follower's and leads
  nothing with it, so panes never chase each other.
- How panes follow and the zoom sync are settings (`sync_scroll`, `sync_zoom` in `settings.toml`); on or off is not.

## Acceptance tests

- [x] `same_position_stays_level_across_different_page_sizes`: halfway down page 3 of four letter pages is the top of
  page 5 of a five-page book of uneven pages, and back.
- [x] `same_page_keeps_the_place_on_the_page_and_stops_at_the_last`
- [x] `pairs_count_on_from_the_nearest_pair`
- [x] `marks_pair_the_panes_open_at_the_time`
- [ ] Tried by hand: two documents of different sizes, each way of following, and the zoom.

## Open

- Not tried by hand.
- Page pairs are kept until cleared or the app quits, not per file: changing a pane's document keeps them.
- Facing and book layouts measure position by single page heights, which drifts a little on spreads.
