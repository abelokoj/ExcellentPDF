# Ph1: 53/60 | F.1: Find in document

**Tag:** F.1; **Priority:** P0; **Phase:** 1; **Needs:** K.13; **Code:** `ecclesia-features::search::find`,
`ecclesia-canvas-win::canvas::find`, `ecclesia-app::find`
**Plan:** §27 entry `Ph1: 53/60 | F.1` and §11.

## Problem

A reader can't find a word in the open document: every reader has Ctrl+F, and without it a long book is hard to use.

## User story

As a reader, I want to type a word and step through every place it appears, with the words round each, so I can find
the passage I am after.

## UI sketch

- **Ctrl+F** (palette: *Find in document*) opens the find bar, a row above the pane's pages: the field, three
  toggles (**Aa** match case, **ab|** whole words only, **é** match accents), the count ("3 of 120", "Searching…",
  "No results"), and previous (↑), next (↓) and close (×) buttons.
- Typing searches at once, from the reader's page round to it; the first hit after the reader's place is shown as soon
  as its page is read. **Enter** or **F3** goes to the next hit, **Shift+Enter** or **Shift+F3** to the previous one,
  round the ends. **Escape** in the bar closes it and takes the hits off the pages.
- Every hit is drawn on the pages in light yellow, the reader's own in orange. A hit already on screen is not scrolled
  to; one off screen is brought a third of the way down the window.
- The **Results** tab, beside the Inspector, lists every hit in document order, each with its page's label and the words round it,
  the hit in bold; pressing one goes there. Its empty state teaches Ctrl+F.
- The bar hides in full screen. Every control has an accessible name; previous, next and close say their shortcuts.

## Mode

All modes, hidden until Ctrl+F.

## Speed and size impact

- Startup: nothing; the bar and the search exist only once opened.
- Pages are read four at a time on the render workers at `Priority::Nearby`, below the screen's tiles and text, so
  scrolling stays first. Time to the first hit on a 1,000-page book is **not measured** (K.24).
- Nothing to the installer.

## Design

- Model (`search::find`, pure Rust): `Query` with `Options {case, words, marks}`; matching over K.13's
  `TextLayout`, with a line break read as a space, a word hyphenated across lines read whole, and whitespace runs
  collapsed; folding of case and of Latin accents and ligatures (æ, œ, ß, fi, fl, ff); `snippet` of about 40
  characters each side, cut at words; `page_order` from the reader's page round; `Hits` in document order with
  `step` round the ends.
- Matching is our own over K.13's text rather than `fz_search_stext_page_cb`, so whole words and accent folding are
  options (see `dev notes/dev_decisions.md`, DD5).
- Canvas (`canvas::find`): runs the search, draws the hits, scrolls to the shown one and sends `found`
  (`{id, searched, pages, total, current, hits}`); commands `find`, `findNext`, `findPrevious`, `findShow`,
  `findClear`. A reflowed book is searched again once laid out.
- App (`ecclesia-app::find`): `FindState` per pane (query, options, progress, hits, the search's id so a late
  answer to an old query is dropped); commands `find.open` (Ctrl+F), `find.next` (F3), `find.previous` (Shift+F3),
  `find.close`; the `results` sidebar panel.
- Not built: the result list shows the first 500 hits (the count and F3 reach all of them); the hits are not added
  to the back/forward history; accent folding covers Latin letters only; the text cache of the pages on screen is
  not reused by the search; no settings.

## Acceptance tests

- [x] `case_whole_words_and_accents_match_as_chosen`: each option changes what matches
- [x] `a_line_break_is_a_space_and_a_hyphenated_word_is_whole`: hits across lines
- [x] `a_snippet_cuts_at_words_round_the_hit`: the words round a hit
- [x] `pages_are_searched_from_the_reader_s_page_round_the_end`: search order
- [x] `hits_step_round_the_ends_in_document_order`: F3 and Shift+F3
- [x] `reads_find_options_and_hits`: the canvas reads the app's commands
- [x] `hits_arriving_out_of_order_are_kept_in_document_order`: the result list
- [ ] first hit on a 1,000-page book within 100 ms when the page is near: **not measured** (K.24)
- [ ] the UI never waits: by design (workers, no waits on the UI thread); **not measured**
