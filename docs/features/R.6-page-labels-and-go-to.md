# Ph1: 17/60 | R.6: Page labels and go to page

**Tag:** R.6; **Priority:** P0; **Phase:** 1; **Needs:** K.3; **Code:** `excellent-features::read::goto`,
`excellent-mupdf::Document::page_labels`, `excellent-app::goto`
**Plan:** §27 entry `Ph1: 17/60 | R.6`.

## Problem

Books number their front matter i, ii, iii. A reader told to "see page iv" had to work out that it is the fourth page,
and had no way to type a page at all.

## User story

As a reader of a book, I want to type "iv" or "37" and land on that page, so that I can follow a reference without
counting.

## UI sketch

- Title bar: the reader's page, "iv (4 of 210)", or "4 of 210" when the label is the number. It is a button (accessible
  name "Go to page; showing page …") that opens the go-to box. Hidden while no document is open.
- Go-to box (Ctrl+G, palette **Navigate › Go to page…**): a field with "of 210" beside it, and one line under it that
  says what Enter will do ("Enter goes to page iv (4 of 210)"), or, in the error colour, that no page has that name.
  Enter goes and closes; Escape closes.
- Palette: First page (Ctrl+Home), Last page (Ctrl+End).

## Mode

Every reading mode, visible whenever a document is open.

## Speed and size impact

Labels are read once per open from the page tree, with no page loaded. The indicator updates only when the page
showing most changes.

## Design

Model: `Pages { count, labels }` finds a label first (ignoring case), then a number. The canvas emits `pages`
(`{count, labels}`) and `page` (`{index}`); the app resolves the input and sends `goToPage` (`{index}`). Jumps are
remembered in the history (R.8).

## Acceptance tests

- [x] `typing_a_roman_label_finds_its_page`: typing "iv" in a book with roman-numbered front matter finds the right page.
- [x] `a_label_wins_over_a_number_and_numbers_fill_the_rest`.
- [x] `page_labels_are_read_and_absent_labels_are_none`: labels come from MuPDF; a PDF without labels goes by number.
- [ ] By hand: in the app, Ctrl+G, type "iv", Enter.
