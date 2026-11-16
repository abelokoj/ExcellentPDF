# Ph3: 19/49 | A.6: Text boxes and typewriter

**Tag:** A.6; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::freetext`,
`excellent-core::annot` (`TextStyle`, `Boxed::Typewriter`), `excellent-mupdf-sys/csrc/shim.c`, `excellent-app::annotate`
**Plan:** §27 entry `#### Ph3: 19/49 | A.6`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Readers write on pages: a comment in the margin, an answer typed into a form that has no fields. Highlights and
sticky notes keep text out of sight; this text must be on the page and look the same in every reader.

## User story

As a reader, I want to put a box of text, or plain typed text, anywhere on a page, in any language, and choose its
font, size, alignment and colour, so that what I write is on the page where it belongs and Acrobat shows it too.

## UI sketch

- `annotate.text_box` (`T`) and `annotate.typewriter` (`Shift+T`), in the palette and the Annotate toolbar: the
  inspector says "Click where the text goes"; the next click places it and opens its text. The command again calls
  it off.
- The inspector, for a text box: "Text box on page 2"; the text; Font (Sans, Serif, Typewriter); Size (9 to 24 pt);
  Alignment; Colour (black, blue, red, green); Border (a switch, boxes only); Delete text box and Done.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

None at startup. Placing reads the clicked page's size once; MuPDF draws the text when the box changes.

## Design

- **Model** (`annotate::freetext`, pure): `Kind`, `place`, `set_text`, `set_style`, `set_border`; each fits the box to
  its text from the same top left (a box keeps its width and wraps; typewriter text fits its longest line).
- **Core**: `TextStyle` and `Font` (the three standard fonts), `Boxed::Typewriter`, `Annotation::text`, through the
  log's JSON (older logs read as the default style).
- **Shim**: the props' `intent`, `quadding`, `font_size`, `text_color`; `EX_ANNOT_FONT`; writing sets `/DA` (only
  when changed), `/Q` and `/IT` for free text only; no popup for a text box.
- **App**: `Placing` (sticky note or text) arms the next `pagePress`; the note editor shows a text box's text and its
  controls; an empty box just placed is deleted when the reader is done.

## Acceptance tests

- [x] `typewriter_text_keeps_its_style_through_save_and_reopen` (Chinese text; `/IT/FreeTextTypeWriter`, `/Cour 14 Tf`
  and `/Q 1` in the bytes, read back).
- [x] `random_annotation_edits_survive_save_and_reopen` (now with typewriter text and text styles).
- [x] `a_text_box_is_placed_at_the_click_with_a_border_and_typewriter_text_without`, `typing_fits_the_box_to_the_text`.
- [x] `chinese_text_is_measured_a_full_em_a_character`, `a_style_or_border_change_is_one_change_only_when_it_changes`.
- [ ] Text boxes typed in Chinese with an input method save and show in Acrobat: not yet checked by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- The text is typed in the inspector, not on the page: in-place editing waits for U.8.
- Boxes are fitted from average character widths, not the fonts' metrics, so a box can be a little wide.
- No background colour control, and no settings for new boxes' style (SumatraPDF's `FreeText*` settings) yet.
- A text box can't be moved or resized yet (A.13).
