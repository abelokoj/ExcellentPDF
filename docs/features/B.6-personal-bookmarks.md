# Ph1: 36/60 | B.6: Personal bookmarks

**Tag:** B.6; **Priority:** P0; **Phase:** 1; **Needs:** K.20, K.28; **Code:** `excellent-features::bookmarks`,
`excellent-store::bookmarks`, `excellent-app::bookmarks`
**Plan:** §27 entry `Ph1: 36/60 | B.6`.

## Problem

The outline belongs to the document and only a PDF can hold one. A reader also needs places of their own, with a
note, in any format, that never change the file.

## User story

As a reader, I want to bookmark the place I am reading, name it, add a note, a colour and tags, and find it again by
searching, so that I can come back to it, even after the file is renamed or moved.

## UI sketch

- Ctrl+D (⌘D) asks the pages where the reader is (the selection, else the top left of the view, and the zoom when it
  is a set one) and opens the editor: title (the selected text, else "Page n", selected to type over), note, tags
  (comma-separated) and colour (the tab groups' eight named swatches). Return in the title or **Add** saves.
- The sidebar's **Bookmarks** tab: a search field (every word must be in the title, note or tags) over the rows, in
  reading order. A row has the colour's dot, the title, the note and tags on lines of their own when there are any, and
  the page's label; a press selects it and goes to its point at its zoom.
- Each row's context menu: Edit bookmark, Delete bookmark. F2 and Delete do the same while the list has the
  keyboard; Escape gives it back to the pages. Delete asks first, since the note goes too.
- No bookmarks yet: what they are, the shortcut, and **Bookmark this place**.

## Mode

Every format and reading mode: bookmarks live in the library (K.20), not the file.

## Speed and size impact

None at startup: the list is read when the tab shows or another file is shown, with one indexed query. Ctrl+D waits
on the library's writer for the new id (well under a millisecond while it is idle; not measured).

## Design

- Migration 4 adds `file.content` (`FileId::content_key`: the size and a hash of the first and last 64 KB) and the
  tables `bookmark` (page, x, y, zoom, quote, title, note, colour, created) and `bookmark_tag`.
- `find_again`, as a file is shown: it records the file's content, and a file with no bookmarks takes those of a file
  with the same content whose path is gone (renamed or moved). A copy beside the original takes nothing.
- The canvas's `outlineTarget` became `readerPlace`, shared by the outline (B.1, B.4) and bookmarks, each with its own
  "waiting" flag; the new canvas command `goToPoint` `{page, x, y, zoom}` jumps to a point and records the jump (R.8).
- `quote` keeps the selected text for re-anchoring by text when the file changes (K.28).

## Acceptance tests

- [x] `bookmarks_are_listed_in_reading_order_with_their_details`: add, edit (tags each once), delete.
- [x] `a_renamed_file_finds_its_bookmarks_by_its_content`: a rename finds them; a copy and other content don't.
- [x] `one_file_has_one_identity_and_new_content_a_new_one`: a copy has the same content key, another path key.
- [x] `a_search_finds_every_word_in_the_title_note_or_tags`, `tags_are_read_as_typed_and_written_back_the_same`.
- [x] `a_reported_place_keeps_its_point_zoom_and_selected_text`.
- [ ] Survives a re-typeset version (K.28, Phase 3: re-anchoring by text is not built; the quote is saved for it).
- [ ] A file edited and then renamed before it is shown again keeps its bookmarks under the old name until K.28.
- [ ] Ctrl+D, the panel, the editor and goToPoint, checked by hand in the app (not run here).
