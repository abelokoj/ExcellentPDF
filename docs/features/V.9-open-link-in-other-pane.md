# Ph3: 8/49 | V.9: Open a link in the other pane

**Tag:** V.9; **Priority:** P1; **Phase:** 3; **Needs:** V.1, R.7; **Code:** `ecclesia-app::panes::elsewhere`, the
page canvas's `openElsewhere` event (`ecclesia-canvas::page`), `TabState::show_then`
**Plan:** §27 entry `#### Ph3: 8/49 | V.9`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Following a footnote, a citation or a cross-reference loses the reader's place: the page jumps, and coming back
means Back or scrolling. Reading a paper with its references, or a chapter with its figures, wants both in view.

## User story

As a reader, I want to open a link, an outline entry or a search result in the other pane, so that I see where it
goes without losing my place.

## UI sketch

- Pages: Ctrl+click (Command-click on a Mac) or middle-click on a link. Footnote and citation markers are links, so
  they open the same way.
- Outline: the middle button on an entry, Ctrl+Enter while the outline has the keyboard, "Open outline entry in
  other pane" in the entry's menu and the command palette. Ctrl+click keeps marking entries (B.10).
- Find results: the middle button on a hit.

## Mode

All modes.

## Speed and size impact

Nothing runs until a link is opened this way. No new dependency.

## Design

- The canvas reports the link's place (`openElsewhere`: `{page, x, y}`) instead of jumping; a link out of the
  document is followed as a click would follow it.
- The other pane is the active one when the click was in another pane, else the next in reading order. With one
  pane, a new pane opens on the right (V.1).
- That pane shows the document's tab if it has one, else a copy of the clicked pane's tab, then goes to the place
  with `goToPoint` or `goToPage`, which leave a history step (R.8). A tab that has to open first keeps the command
  until its file is open (`TabState::show_then`, sent after the view comes back).
- The clicked pane is never sent a command, so its reading position stays.

## Acceptance tests

- [x] `a_link_place_reads_its_point_only_when_it_has_one`: the canvas's event becomes a place, and the place the
  command that goes there.
- [ ] Tried by hand: the reading position in the source pane doesn't move, with one pane and with two.

## Open

- Not tried by hand.
- A search result opens at its page, not at the hit: the other pane has no find results of its own.
- Ctrl+click on a search result shows the hit here, as a plain click: NSH 0.5's button hears presses without their
  modifier keys.
- With one pane, the new pane becomes the active one, as a split always does.
