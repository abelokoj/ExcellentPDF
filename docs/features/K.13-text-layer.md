# Ph1: 3/60 | K.13: Text layer and selection

**Tag:** K.13; **Priority:** P0; **Phase:** 1; **Needs:** K.3; **Code:** `crates/excellent-text`,
`excellent-mupdf::text`, `excellent-core::render::read_text`, `excellent-canvas-win`
**Plan:** §27 entry `Ph1: 3/60 | K.13`, Appendix D.1.3.

## Problem

Text on a page could not be selected or copied, and the features built on text (focus mode, the ruler, search, read
aloud, reflow) had nothing to read it through.

## User story

As a reader, I want to drag across text, or double- or triple-click it, and copy what I selected, so that I can quote
a document anywhere.

## UI sketch

- Over text the pointer is an I-beam. A drag selects characters; a double click a word, a triple click a line, a
  fourth a paragraph, and dragging after them grows by the same unit. A selection can run across pages.
- The selection is a light blue band per line, drawn so black text stays black under it. A press away from text
  clears it.
- **Copy selected text** (`Ctrl+C`, `Ctrl+Insert`) and **Select all text on this page** (`Ctrl+A`) are in the
  command palette's Edit group. A text field with the focus keeps its own copy and select all.

## Mode

Every reading mode. The pen still draws; the mouse selects.

## Speed and size impact

Nothing runs at startup. A page's text is read on the render workers when the page comes on screen (below the pages'
tiles), kept while the page stays within eight pages of the screen, and read for every selected page so a copy has
all of it. Hit testing and drawing a selection walk one page's lines and characters on the UI thread. Not measured.

## Design

- `excellent-mupdf`: the shim copies the structured text into arrays (blocks, lines, characters with quads), so the
  page's text crosses threads as plain data (`TextLayout`).
- `excellent-text` (pure Rust, no OS types): `PageText` (caret and nearest character under a point, `unit` for word,
  line and paragraph, `highlight`, `text`), `Selection` (anchor, focus, granularity; per-page ranges; text across
  pages) and word classes.
- `excellent-canvas-win`: a near-pages cache (`near.rs`, shared with links), click counting (`clicks.rs`), mouse
  capture while selecting, the highlight geometry (`Gfx::highlight`), and the `copy` and `selectAll` commands; the
  `copy` event carries the text to the app, which puts it on the clipboard (NSH `set_clipboard`, the `clipboard`
  capability).
- Deviation from the plan's step 4, recorded there: selection runs over the copied quads, not MuPDF's selection
  functions, because a MuPDF text page can't leave its worker.

## Acceptance tests

- [x] `the_caret_falls_between_the_characters_either_side_of_the_pointer`, `the_nearest_character_is_the_one_under_the_pointer`,
  `the_pointer_is_over_text_on_a_line_or_just_beside_it`.
- [x] `words_lines_and_paragraphs_snap_around_a_character`, `letters_digits_and_inner_joiners_make_words`.
- [x] `a_highlight_is_one_quad_per_line`, `copied_text_breaks_lines_and_separates_blocks`,
  `a_word_hyphenated_across_lines_is_copied_whole`.
- [x] `copying_a_page_gives_the_same_words_as_mupdf` (zlib manual, page 1), `a_text_layout_covers_its_characters_in_order`.
- [x] `a_drag_selects_between_its_carets_either_way`, `a_double_click_drag_grows_by_whole_words`,
  `a_selection_across_pages_takes_all_of_the_pages_between`, `clicks_choose_the_granularity`.
- [x] `quick_presses_in_one_place_count_up`, `the_message_clock_may_wrap`.
- [ ] Selection and copy on the text-quality corpus score at or above `pdftotext` (step 5): needs the corpus.
- [ ] A double click selects a word in Chinese and Thai text: needs ICU4X segmentation (step 3).
- [ ] Reading order from structure tags and columns (step 2); rectangle selection; drag past the edge scrolls.
- [ ] Checked by hand on the Surface Pro: drag, double and triple click, copy into another app.
