# Ph1: 30/60 | R.28: Other formats through MuPDF

**Tag:** R.28; **Priority:** P0; **Phase:** 1; **Needs:** K.3; **Code:** `ecclesia-core::formats`,
`ecclesia-mupdf` (`Document::is_reflowable`, `Document::reflow`), `ecclesia-core::document` (`Document::reflow`),
`ecclesia-canvas-win` (laying books out to the window)
**Plan:** §27 entry `Ph1: 30/60 | R.28`.

## Problem

Readers keep more than PDFs: EPUB and MOBI books, FB2, comics in CBZ, XPS, SVG, pictures, saved web pages and
Markdown notes. A second app for each splits their reading in two. Books have no pages of their own, so a reader that
shows them at one fixed size wastes a large window or crowds a small one.

## User story

As a reader, I want to open my books, comics and pictures in the same app as my PDFs, with a book's text at the size
I chose and its pages filling my window, so that I read everything in one place and never lose my page when I resize
the window or change the text size.

## UI sketch

- The Open dialog's "Documents" filter, a drop on the window and "Set as default PDF viewer" all take the new types.
- A book (EPUB, FB2, MOBI, HTML, Markdown) opens with each page one window tall and as wide as the window, up to a
  comfortable line length (36 ems), shown at its size, so the text is at the reader's size. Resizing the window, or
  changing the size, lays the book out again in the background; the reader stays on the text they were reading.
- Settings, Reading: "Book text size", 8 to 32 pt (12 by default), with the other reading settings.

## Mode

All modes.

## Speed and size impact

- Startup: nothing.
- A book is laid out on the thread that opens it, before its first page shows. Laying it out again runs on a thread
  of its own; changes while it runs are merged into one more layout. Not measured yet (K.24).
- Installer: none. Every format is MuPDF's, already in the build (plan D3).

## Design

- `formats` (pure): the extensions, the signature of each format (PDF anywhere in the first kilobyte, ZIP, PalmDB,
  XML and HTML, the picture formats; Markdown by its name, when it is text), and the page size for a window and a text
  size.
- `ecclesia-mupdf`: `is_reflowable`, and `reflow(size, keep)`, which takes `&mut self` so no page is loaded while
  the layout replaces them. The shim makes a bookmark (a place in the text) of `keep`'s start, lays the book out, and
  finds the bookmark's page.
- `ecclesia-core::Document::reflow`: lays the book out once for all its views, clears its display lists and raises
  every page's revision, so stale lists are refused. It remembers the size to lay the book out again after
  hibernating. A layout is not an edit: no unsaved changes.
- The canvas: lays a book out for its window when opening it, shows it at zoom 1, and lays it out again on a resize or
  a new text size, then forgets the old pages' tiles, text, links and history, and scrolls to the kept page.
- The setting: `reflow_font_size` in `settings.toml`, the canvas's `reflowFontSize` prop.

## Acceptance tests

From the entry's **Done when** ("every format in the corpus opens; font size changes keep the reading position in
EPUB"):

- [x] `every_corpus_file_renders_page_one_to_png_and_gives_text`: one corpus file per format (EPUB, FB2, MOBI, HTML,
  Markdown, SVG, CBZ, PNG, XPS), each opened and drawn; text read from all but the pictures.
- [x] `reflowing_keeps_the_reading_position`: EPUB, FB2, MOBI, HTML and Markdown at 9 pt, then 18 pt from a middle
  page: more pages, and the page's first words on the kept page.
- [x] `a_reflow_changes_every_page_and_survives_hibernating`: stale lists refused, no unsaved changes, the same
  layout after waking.
- [x] `files_are_known_by_their_contents`, `markdown_goes_by_its_name_and_must_be_text`,
  `every_extension_is_lower_case_and_listed_once`, `reflowed_pages_fill_the_window_up_to_a_readable_line`.
- [x] `dropped_files_are_taken_by_their_contents_not_their_names`: drops of the corpus's books.
- [ ] Tried by hand: a real EPUB and comic, resizing the window, changing the text size in Settings.
- [ ] Two views of one book share its layout, so the last laid out wins (V.1's split views).
- [ ] DjVu, CHM and CBR: R.29.
