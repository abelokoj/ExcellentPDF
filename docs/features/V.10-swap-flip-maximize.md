# Ph3: 9/49 | V.10: Swap, flip and maximize

**Tag:** V.10; **Priority:** P1; **Phase:** 3; **Needs:** V.4; **Code:** `ecclesia-app::panes` (`Layout::flip`,
`Layout::toggle_maximized`, `Panes::swap`), the pane header in `panes::grid`
**Plan:** §27 entry `#### Ph3: 9/49 | V.10`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Once the window is split, the panes sit where they were opened. Putting the reference on the other side, turning
side by side into stacked for a wide page, or giving one pane the whole window for a moment means closing and
opening panes again.

## User story

As a reader with two documents open, I want to swap the panes, flip them between side by side and stacked, and
maximize one for a moment, so that the layout suits what I am reading without losing either document.

## UI sketch

- Palette and shortcuts: "Swap panes" (Ctrl+Alt+X), "Flip panes between side by side and stacked" (Ctrl+Alt+F),
  "Maximize pane or restore panes" (Ctrl+Alt+Enter); all three are in the toolbar's customization list.
- Pane header: a double-click maximizes the pane or brings the others back. In a split the header stands for the
  plan's "its tab": the tab strip is the pane's own and a double-click there is taken. A □ button before the close
  button does the same, read out as "Maximize pane" or "Restore panes".

## Mode

All modes.

## Speed and size impact

None until used: the layout gains one field, and the pane views are shown or hidden, never rebuilt, so a maximized
pane's neighbours keep their canvases and come back as they were.

## Design

- Swap: the active pane's tabs (order, groups, views) and the next pane's trade places with `TabState::replace`;
  the active cell stays, so swapping again undoes it.
- Flip: `Layout::flip` swaps the top right and bottom left cells, the active and maximized cells with them, and the
  rows' heights with the top row's widths; the two cells' panes trade tabs, so every document stays with its pane.
  Twice is the identity (tested).
- Maximize: `Layout::maximized` hides the other panes (`is_open` is false for them) without closing them. Closing,
  splitting and a pane running out of tabs bring them back first, so nothing hides behind a single pane.
- Each is one layout change; doing it again is its undo. Document undo (K.14) does not hold layout changes.

## Acceptance tests

- [x] `swap_flip_and_maximize_each_undo_themselves`: flip twice and maximize twice give the layout back; while
  maximized, only the active pane shows and splitting finds the hidden panes' cells taken.
- [ ] Tried by hand: swapping, flipping and maximizing with two and four panes, and that each pane keeps its page.

## Open

- Not tried by hand.
- A double-click on the header's Sync, □ or × button may also count as a double-click on the header.
