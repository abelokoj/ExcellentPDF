# R.5 Thumbnails

**Tag:** R.5 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.7 · **Code:** `excellent-features::read::thumbnails`,
`excellent-core::thumbnail`, `excellent-app::thumbnails`
**Plan:** §27 entry `#### R.5`.

## Problem

A reader had no overview of the document and no way to reach a page by its look.

## User story

As a reader, I want to see the pages small in the sidebar, with their labels, so that I can find a page by sight and
go there with one press.

## UI sketch

- The left dock's tab **Pages** (Toggle sidebar, F12, opens and closes it): one column of pages, each a white sheet
  with a hairline border and a soft shadow, shaped like its page, its label ("iv", "12") under it.
- The reader's page is ringed in the theme's ring colour and its label is in the foreground colour, semibold; the
  ring is always there, clear on other pages, so nothing moves when the page changes.
- Hover tints the row; a press goes to the page. Each row is a button named "Page iv (4 of 210)", with ", the page
  you are reading" on the reader's page.
- Without a document the tab says "Open a document to see its pages here".

## Mode

Every reading mode.

## Speed and size impact

Only the rows near the list's viewport exist (NSH `virtual_list`), and only they, with 6 more each way, are rendered,
at the workers' thumbnail priority, below the tiles on screen and the nearby pages; rows that scroll away cancel their
renders. A render reuses the page's display list when the canvas recorded one, and keeps the one it makes for the
canvas. Each thumbnail is a 320-pixel-wide PNG in the app's data folder, named by the file's identity, the page, its
revision and the width, so reopening the file shows them without rendering. Not measured.

## Design

- Model (`excellent-features::read::thumbnails`): `frame` (a page's frame inside the 136 × 176-point box),
  `wanted` (the pages to render for the rows shown), `file_name` (the cache name).
- Rendering (`excellent-core::thumbnail::render`): one job on the shared pool (`excellent-core::shared`, which the
  canvas uses too) that draws the page and writes the PNG atomically.
- The canvas's `pages` event carries each page's size, so frames have their final shape before any render.
- View (`excellent-app::thumbnails`): the list, the renders under way (dropped, so cancelled, when they leave the
  wanted range), and the rendered files.

## Acceptance tests

- [x] `frames_keep_the_page_shape_and_fit_the_box`, `the_rows_shown_and_a_margin_are_wanted`,
  `names_change_with_the_file_the_page_and_its_revision`.
- [x] `a_page_becomes_a_png_file_of_the_asked_width`: a corpus page rendered to a file, its display list kept.
- [x] `starts_with_the_document_between_pages_and_inspector`, `shortcuts_toggle_the_docks`: the Pages tab.
- [ ] A 5,000-page document's thumbnail list scrolls at 60 fps and never delays visible tiles: needs the budget checks
  (K.24) and a run on the reference machine.
- [ ] The list follows the reader's page as it changes: needs a way to scroll the list to a row.
- [ ] The cache folder is trimmed: K.20.
