# Ph3: 6/49 | B.10: Sort, promote and demote between outline and bookmarks

**Tag:** B.10; **Priority:** P1; **Phase:** 3; **Needs:** B.1, B.6; **Code:** `excellent-features::outline`
(`plan_sort`, `sort_level`, `spot`, `Spot`), `excellent-app::outline`, `excellent-app::bookmarks`, `excellent-app::shell`
**Plan:** §27 entry `#### Ph3: 6/49 | B.10`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Outlines made by hand or by tools end up out of page order, and the reader's own bookmarks and the document's outline
live apart: a place worth sharing with every reader of the file is stuck in one person's bookmarks, and an outline
entry the reader wants to annotate with notes and tags can't be.

## User story

As an editor of a document, I want to put an outline level in page order in one step, and move a place between my
bookmarks and the document's outline without retyping its title or finding the spot again.

## UI sketch

- Outline entry menu and palette: "Sort outline entries by page" sorts the selected entry's children, or the level it
  is on when it has none. Entries that open no page go last; equals keep their order. One step of undo.
- Outline entry menu and palette: "Add outline entry as bookmark" makes a personal bookmark with the entry's title,
  page, point and zoom, shown selected in the Bookmarks pane. It works on read-only documents too.
- Bookmark menu and palette: "Add bookmark to outline" makes an outline entry with the bookmark's title and place,
  after the entry the place's page is in, selected. One step of undo; needs an editable PDF.

## Mode

All modes.

## Speed and size impact

- Nothing at startup. Sort is a walk of one level; the hand-over reads the outline once, on the UI thread, as the
  other outline edits do.
- No new dependency.

## Design

- Both hand-overs copy and keep the original: a bookmark holds notes, tags and a colour the outline can't, and taking
  an entry out of a document is an edit to someone else's file. Deleting the original stays one step away.
- `plan_sort` gives moves (`MoveOutlineEntry`) rather than a rewrite, so each entry keeps its children, style and
  destination untouched.
- `spot(uri)` reads where a destination opens from MuPDF's link format: `#page=N&zoom=Z,X,Y` (Z in percent; NaN means
  the reader's own) and `view=FitH,Y` / `FitV,X`. Named destinations open at the page's top-left.
- The shell routes the two hand-overs, since each needs the other pane's state; neither pane reaches into the other.

## Acceptance tests

- [x] `a_place_survives_the_trip_into_the_outline_and_back`: a place (page, point, zoom) written as an outline entry
  reads back with the same title, page, point and zoom.
- [x] `a_level_sorts_by_page_with_equals_kept_and_nowhere_last`
- [x] `a_link_gives_its_spot`
- [ ] Tried by hand: sort a shuffled outline; hand an entry each way.

## Open

- Not tried by hand.
- A named destination's own point isn't read: its bookmark opens at the page's top.
