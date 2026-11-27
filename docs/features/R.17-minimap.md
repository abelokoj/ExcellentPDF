# Ph3: 2/49 | R.17: Mini-map with note ticks

**Tag:** R.17; **Priority:** P1; **Phase:** 3; **Needs:** R.5, K.14; **Code:** `ecclesia-features::read::minimap`,
`ecclesia-canvas::page::canvas::minimap`, `ecclesia-core::annot::page_marks`
**Plan:** §27 entry `#### Ph3: 2/49 | R.17` and §11.2. The label is the entry's place in its phase (plan §27.0, Tag
rules).

## Problem

A long document gives no sense of where the reader's own work is. The scrollbar says where the screen is, but not
where the highlights, notes, bookmarks and search hits are, so going back to them means paging through.

## User story

As a reader with a heavily annotated 300-page book, I want a strip beside the pages that shows where my marks and
search hits are, and that I can press or drag to get there, so that I can move through the document by its content.

## UI sketch

- A 12 px strip (at 96 dpi) on the right edge of the pages, inset 2 px. It always stands for the whole document,
  top to bottom, whatever the layout: in a horizontal right-to-left layout the first page is at the top too.
- One faint tick where each page (each spread, in a two-page layout) starts; ticks closer than 3 px are left out.
- A translucent thumb, at least 16 px tall, over the part on screen.
- Marks in lanes, so they don't hide each other: highlights on the left, notes in the middle, search hits on the
  right, bookmarks across. Each is drawn in its own colour (a highlight's own, a bookmark's colour chip), deepened on
  light backgrounds so it reads; the current search hit is orange and at least 4 px tall. Touching marks of the same
  kind and colour merge into one.
- Pressing the strip centres the screen there; pressing the thumb and dragging keeps the grip. In paged flow a press
  moves to the page there first.
- The strip shows only when it has something to say: hidden when the view is narrower than eight strips, and when the
  whole document is on screen with nothing marked.
- Keyboard and screen readers: the strip is a pointer shortcut only. Every place it reaches is reachable from the
  keyboard (Page Up and Down, Go to page, F3 for hits, the bookmark and annotation lists), so it has no focus stop of
  its own.

## Mode

All modes, visible by default.

## Speed and size impact

- Startup: nothing. Marks are read when a document is shown, in one background job over every page, at the lowest
  priority, so rendering goes first.
- After an edit only the changed pages are read again, on the UI thread before the next frame (one page's
  annotations: well under a millisecond on the corpus).
- Drawing is a handful of rectangles per frame.
- No new dependency.

## Design

Model, `ecclesia-features::read::minimap`, pure Rust:

- `Scale` maps the layout's along-axis extent (first band to last) onto the strip, forward or reversed; it gives the
  thumb, the ticks, the placed marks and where a press or drag scrolls to.
- `Palette::for_background` derives the track, tick and thumb colours from the page-area colour, and `ink` deepens a
  mark's colour on light backgrounds.

Core:

- `annot::page_marks` reads a page's annotation kinds, rects and colours (not links, pop-ups or form fields);
  `Renderer::load_marks` reads a range of pages as a cancellable job.
- An operation now knows its pages (`Operation::pages`); applying it, undoing and redoing bump those pages'
  revisions (and drop their display lists), and the history remembers each operation's pages.

View, `ecclesia-canvas::page::canvas::minimap`:

- Holds the marks per page with the revision they were read at; before each frame, pages whose revision moved are read
  again, so a mark follows an annotation change within one frame.
- Search hits come from the find state (F.1); bookmarks from the `bookmarks` prop, which the app fills from the
  library (B.6) whenever a document is shown.

No command, no setting yet.

## Acceptance tests

- [x] `an_annotation_edit_changes_its_page_and_its_marks`: an edit, its undo and its redo each bump that page's
  revision and change its marks; the frame after a revision bump reads them again (Done when: marks update within one
  frame of an annotation change).
- [x] `the_strip_stands_for_the_whole_document_either_way_round`
- [x] `the_thumb_covers_the_screen_and_stays_big_enough_to_grab`
- [x] `ticks_too_close_together_are_left_out`
- [x] `marks_keep_their_lanes_and_touching_ones_merge`
- [x] `pressing_centres_the_screen_and_dragging_keeps_the_grip`
- [x] `the_palette_follows_the_background`
- [x] `reads_bookmarks_and_leaves_out_the_broken_ones`
- [ ] Tried by hand: pressing, dragging, marks after a highlight, in each layout.

## Open

- Not tried by hand.
- No setting to hide it.
- The strip is 12 px wide, under the 24 px pointer target of WCAG 2.5.8; it is a shortcut with keyboard
  equivalents, which the criterion allows, but a wider hover target may be worth trying.
