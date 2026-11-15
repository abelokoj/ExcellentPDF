# Ph3: 5/49 | B.9: Check destinations

**Tag:** B.9; **Priority:** P1; **Phase:** 3; **Needs:** B.1; **Code:** `excellent-features::outline` (`broken`,
`check`, `search_from`, `heading_top`), `excellent-core::outline::find_headings`, `excellent-app::outline`
**Plan:** §27 entry `#### Ph3: 5/49 | B.9`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Outlines go stale: pages are deleted, files are merged or split, named destinations are dropped by tools. A broken
entry silently does nothing when pressed, and nothing tells the reader which ones are broken.

## User story

As a reader or editor of a long document, I want to see which outline entries open nowhere and fix them in one go, so
that the outline can be trusted.

## UI sketch

- A broken entry says why at the end of its row, in place of its page label, after a "!" mark and in the destructive
  colour (kept at WCAG AA on every row background): "No destination", "Destination missing" or "No page 412".
- "Check outline destinations" (entry menu and palette) counts them: "No broken entries" or "Broken outline entries:
  3", with what Fix does. It runs on read-only documents too.
- "Fix broken outline entries" (entry menu and palette) looks on the workers for the heading with each broken entry's
  title, from the page of the nearest working entry before it, then points the entry at the heading, or removes it when
  none is found: one step of undo, with a message saying how many of each.

## Mode

All modes. Fix needs an editable PDF, as B.1 to B.5.

## Speed and size impact

- The check is a walk of the outline already in memory, done as rows draw; nothing at startup.
- Fix reads page text once per page on the workers at background priority, stopping when every heading is found; it
  is cancelled if the outline goes.
- No new dependency.

## Design

- `broken(uri, page, page_count)`: no destination at all; an internal destination (`#...`) MuPDF couldn't resolve; a
  page past the last. Links out of the document (web pages, other files) are not broken.
- `heading_top(layout, title)`: a line, or a block's lines together, reading as the title once case, punctuation and
  spacing are folded; the largest type wins.
- `find_headings` reads each page's text once for all broken entries; it prefers the first match at or after an
  entry's search page, else the first before it.
- The fix is checked against the outline it started from: if the outline changed meanwhile, nothing is done.

## Acceptance tests

- [x] `every_broken_entry_in_a_test_file_is_found`: a PDF with a missing named destination and an entry with no
  destination has both found; its working entry and web link aren't.
- [x] `every_broken_entry_is_found` (the model: all three kinds, a `#page=N` link past the end, web links)
- [x] `a_heading_is_found_by_its_title`
- [x] `headings_are_found_on_the_workers`
- [ ] Tried by hand: Fix on a file with broken entries.

## Open

- Not tried by hand.
- What MuPDF hides: an entry pointing at a page object no longer in the page tree (a deleted page) resolves to the
  first page, so it isn't flagged; and an entry with a page number past the last (not allowed by the spec, written
  by some tools) makes MuPDF fail to read the whole outline. Both need a MuPDF patch or our own reading of `/Dest`.
- "Nearest heading" is the heading with the entry's title; B.8 (Phase 5) brings real heading detection, which can
  replace it.
