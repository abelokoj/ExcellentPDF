# Ph2: 3/3 | U.3: Canvas accessibility on macOS and Linux

**Tag:** U.3; **Priority:** P0; **Phase:** 2 (desktop), 8 (tablets); **Needs:** U.2; **Code:**
`ecclesia-text::flat`, `ecclesia-canvas::page::canvas::access`, `ecclesia-canvas-gtk::access`,
`ecclesia-canvas-mac::access`
**Plan:** §27 entry `Ph2: 3/3 | U.3`; §15.

## Problem

On macOS and Linux the page canvas is a view NSH doesn't describe, so VoiceOver and Orca find nothing to read where
the document is.

## User story

As a reader who uses VoiceOver or Orca, I want to read the pages in order, by line, word and character, and hear and
change the selection, as in any document.

## UI sketch

Nothing new on the screen. On Linux the canvas is a **Document** with GTK's text interface; on macOS it is a
**text area**. Both hold the text of the pages read so far, a line break after each line, in reading order.

## Mode

Every mode.

## Speed and size impact

- Startup: nothing; the text is gathered only when a screen reader asks.
- Budgets: the flat text is made on each request (a shortcut, noted in the code); none at all without a screen reader.
- Size: none.

## Design

- Model: `ecclesia_text::FlatText` lays the document's text out as one string and maps offsets to page positions
  and back (pure, unit-tested). The shared canvas answers in those offsets: units, the selection, the pages shown,
  areas on the screen (`flat_*`).
- Linux: the widget implements `GtkAccessibleText` (GTK 4.14, which NSH uses): contents, units, caret, selection.
  Changes are announced after the canvas is done (an idle callback).
- macOS: the view answers AppKit's text-area methods: value, characters, selection (also set), visible range, lines,
  strings and frames for ranges; changes post `ValueChanged` and `SelectedTextChanged`.
- AccessKit (item 2) was weighed and not used: GTK already exposes the app's AT-SPI tree and NSH the macOS one, so the
  canvas joins each toolkit's own interface rather than running a second provider beside it.
- Pages, links and notes as their own elements (U.2 on Windows) come next on these platforms.

## Acceptance tests

From **Done when**: VoiceOver and Orca read pages in order.

- [x] `lines_end_in_breaks_and_unread_pages_add_nothing`: the flat text is the pages' lines in order.
- [x] `positions_and_offsets_round_trip`: every offset leads back to its page position.
- [x] `lines_count_from_the_document_start`: lines for AppKit's line queries.
- [x] `no_pages_is_empty`: no document, no text.
- [ ] By hand: VoiceOver reads the pages in order on macOS.
- [ ] By hand: Orca reads the pages in order on GNOME.

## Open

Pages, links and notes as elements; sentences (K.13 step 3); UTF-16 offsets for characters outside the BMP on macOS;
trying it with VoiceOver and Orca.
