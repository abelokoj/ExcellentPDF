# B.1 Outline (reading part)

**Tag:** B.1 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.3, K.13 · **Code:** `excellent-features::outline`,
`excellent-core::outline`, `excellent-app::outline`
**Plan:** §11.14, §27 entry `#### B.1`.

This is the outline's reading half, which B.1–B.5 edit. Adding entries (B.1 steps 2–4) waits for the text layer
(K.13) and the operation log and save (K.14, K.15).

## Problem

A reader had no way to see a document's table of contents or jump to a chapter.

## User story

As a reader, I want the document's outline in the sidebar, with the page each entry opens and the entry I'm in marked,
so that I can see where I am and go to any chapter with one press.

## UI sketch

- The left dock's tab **Outline**, beside Pages: the entries as a tree, each level indented, a chevron on entries with
  children (it turns down when they are expanded), the title, and on the right the label of the page it opens.
- A press, or the arrow keys, on an entry goes to its page (and Back returns). Return, Space, Right and Left expand
  and collapse; letters jump by typeahead (NSH `Tree`).
- The entry the reader is in is selected and semibold, and kept in view as pages turn; a chosen entry stays selected
  while it opens the page being read, even when a later entry opens the same page.
- Entries start expanded as the file says. A document without an outline says "This document has no outline".

## Mode

Every reading mode.

## Speed and size impact

Nothing runs at startup: the outline is read on the render workers when the tab first shows (and again for another
document), above thumbnails and below the pages on screen. The tree is virtualized: only the rows in view exist. Not
measured; the 10,000-entry outline of §11.14's tests isn't in the corpus yet.

## Design

- Model (`excellent-features::outline`): `Entry`; ids are paths from the top ("2.1.4"), as the MuPDF outline
  iterator walks them (§11.14 step 4); `find`, `open_ids`, `at_page` (the entry opening the nearest page at or before
  the reader's, the last of those in document order) and `reading` (the chosen entry while it still opens the page).
- Reading (`excellent-core::outline::load`): one job on the shared render pool.
- View (`excellent-app::outline`): NSH `Tree` with controlled expansion and selection; the rows are ours.

## Acceptance tests

- [x] `ids_are_paths_and_find_their_entries`, `the_reader_is_in_the_nearest_entry_before_the_page`,
  `a_chosen_entry_stays_while_it_opens_the_page`.
- [x] `an_outline_is_read_on_the_workers`; `reads_the_outline` (`excellent-mupdf`).
- [x] `starts_with_the_document_between_pages_and_inspector`: the Outline tab is there.
- [ ] An added entry opens at the same place in Acrobat and pdf.js (B.1's **Done when**): needs K.13, K.14, K.15.
- [ ] A 10,000-entry outline stays smooth (§11.14 tests): needs a corpus file and the budget checks (K.24).
- [ ] Hovering an entry previews its destination page (§11.14 UI).
