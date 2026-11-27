# Ph1: 31/60 | B.1: Add an outline entry

**Tag:** B.1; **Priority:** P0; **Phase:** 1; **Needs:** K.3, K.13, K.14, K.15; **Code:** `ecclesia-features::outline`,
`ecclesia-core::outline`, `ecclesia-mupdf::edit`, `ecclesia-app::outline`, `ecclesia-canvas-win` (`outlineTarget`)
**Plan:** §11.14, §27 entry `Ph1: 31/60 | B.1`.

B.1 holds the outline panel (its reading half) and adding entries; B.2 to B.5 edit them.

## Problem

A reader could not see a document's table of contents, jump to a chapter, or add a chapter the file was missing.

## User story

As a reader, I want the document's outline in the sidebar, with the entry I'm in marked, and to add an entry for the
place I'm reading (titled with the text I selected), so that I can find my way through any document.

## UI sketch

- The left dock's tab **Outline**, beside Pages: the entries as a tree, each level indented, a chevron on entries with
  children, the title, and on the right the label of the page it opens.
- A press, or the arrow keys, on an entry goes to its page (and Back returns). Return, Space, Right and Left expand
  and collapse; letters jump by typeahead (NSH `Tree`). The reader's entry is semibold and kept in view.
- **Add outline entry here** (Ctrl+B, Enhanced's CmdAddBookmark; the palette's Outline group; a row's context menu):
  a new entry after the selected one, opening at the top of the selection, or else at the top-left of the view.
  Its title is the selected text (whitespace collapsed, cut at a word near 200 characters), or "Page 12" selected
  in place, ready to type over.
- A document without an outline says "This document has no outline" and offers **Add an entry for this page**; the
  first entry makes the outline.
- A file that isn't a PDF says its outline can't be changed.

## Mode

Every reading mode.

## Speed and size impact

Nothing runs at startup: the outline is read on the render workers when the tab first shows. After an edit the
outline is read again on the UI thread, so the paths an edit uses are never stale; a very large outline could stall
there (not measured; marked in the code as a shortcut).

## Design

- Model (`ecclesia-features::outline`): ids are iterator paths ("2.1.4"); `title_from_selection`, `insert_at`.
- Place: the canvas answers the `outlineTarget` command with `{page, x, y, zoom, title}`; zoom only when it is a
  custom zoom, so a fit mode stays a fit mode. `Document::link_uri` makes the URI with `fz_format_link_uri`.
- Insert: one operation (K.14) through `fz_outline_iterator_insert`.

## Acceptance tests

- [x] `ids_are_paths_and_find_their_entries`, `the_reader_is_in_the_nearest_entry_before_the_page`,
  `a_chosen_entry_stays_while_it_opens_the_page`, `an_outline_is_read_on_the_workers`.
- [x] `paths_titles_and_where_a_new_entry_goes`; `the_first_entry_makes_an_outline` (`ecclesia-mupdf`).
- [ ] An added entry opens at the same place in Acrobat and pdf.js (checked by hand).
- [ ] A 10,000-entry outline stays smooth (needs a corpus file and K.24).
- [ ] Hovering an entry previews its destination page.
