# Ph3: 4/49 | B.7: Bold, italic and color

**Tag:** B.7; **Priority:** P1; **Phase:** 3; **Needs:** B.1; **Code:** `excellent-mupdf-sys` (`ex_outline_style`,
`ex_load_outline`), `excellent-mupdf::OutlineStyle`, `excellent-core::edit::Change::StyleOutlineEntry`,
`excellent-features::outline` (`Swatch`, `Restyle`, `plan_style`), `excellent-app::outline`
**Plan:** §27 entry `#### Ph3: 4/49 | B.7`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Long tables of contents are easier to scan when parts stand out: chapters in bold, appendices in italic, a colour for
the sections being worked on. PDF outlines can carry this (`/F` and `/C`), and Acrobat shows it, but most readers
ignore it and few let you set it.

## User story

As a reader organising a long document, I want to make outline entries bold, italic or coloured, and see the styles the
document already has, so that the outline reads at a glance here and in Acrobat.

## UI sketch

- Rows draw the document's style: bold, italic, and its colour as the title colour. The colour is mixed toward the
  theme's text colour just enough to read at WCAG AA on the sidebar, a hovered or selected row and a marked row, so a
  dark blue entry stays readable in a dark theme; black counts as no colour (as in SumatraPDF Enhanced).
- The entry context menu gains, after "Point outline entry here": "Bold outline entry", "Italic outline entry" and a
  "Colour" submenu (No colour, Red, Orange, Green, Blue, Purple, each with a dot of its colour).
- Ctrl+Shift+B and Ctrl+Shift+I (⌘ on a Mac) toggle bold and italic while the outline has the keyboard. All eight
  commands are in the command palette.
- With several entries marked (B.5), the command acts on all of them as one step of undo; bold and italic turn on for
  all unless all have it, then off.

## Mode

All modes; editing needs a PDF (other formats show the read-only message, as B.1 to B.5).

## Speed and size impact

- Reading the outline walks the PDF outline iterator once beside `fz_load_outline` to pick up `/F` and `/C`; no work at
  startup.
- No new dependency.

## Design

- Shim: `ex_load_outline` fills each `fz_outline`'s `flags` and `r g b` from the PDF outline iterator (which
  `fz_load_outline` leaves out). `ex_outline_item` reports `flags` and `color` (`0xRRGGBB` or -1).
  `ex_outline_style` writes an entry's `/F` and `/C` through `fz_outline_iterator_update`, keeping its title,
  destination and open state.
- `/F` bits follow the PDF spec (1 italic, 2 bold); MuPDF's `FZ_OUTLINE_FLAG_*` names have them swapped.
- `OutlineStyle { bold, italic, color }` is read with every `OutlineItem` and written by `Edit::style_outline_entry`.
- `Change::StyleOutlineEntry` (operation log kind `outline.style`, colour as `#rrggbb` or null) makes it one undoable
  operation saved by K.15.
- The model's `plan_style` turns a command into the changed entries' paths and styles; the app's `readable` keeps the
  drawn colour at AA.

## Acceptance tests

- [x] `a_styled_entry_keeps_its_style_after_reopening`: bold and blue survive undo, redo, save and reopening; the
  saved file has `/F 2`.
- [x] `style_toggles_together_and_skips_what_stays`
- [x] `changes_read_back_as_they_were_written` (with `outline.style`)
- [x] `readable_keeps_a_colour_that_passes_and_mixes_one_that_does_not`
- [ ] Styles show the same in Acrobat (opened by hand).

## Open

- Not opened in Acrobat or tried by hand.
- The swatches are a fixed set of five; there is no custom colour picker.
