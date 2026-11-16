# Ph3: 33/49 | A.20: Stamps library with dynamic fields

**Tag:** A.20; **Priority:** P1; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::stamps`,
`excellent-app::annotate::stamps`, `excellent-core::edit` (`Change::StampArt`), `excellent-mupdf` (`StampArt`,
`Edit::set_stamp_art`, `Page::to_pdf`, `html_to_pdf`)
**Plan:** §27 entry `#### Ph3: 33/49 | A.20`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reviewers mark whole pages: approved, draft, received on a date by someone. Typing that as a text box is slow and
doesn't look like a stamp in other readers.

## User story

As a reviewer, I want to stamp a page "Approved" with my name and the time, or with my own seal, in one click, and
have Acrobat show it the same.

## UI sketch

- **Stamp** (Annotate toolbar, Shift+M on a page, the palette) waits for a click on a page; the command again calls it
  off.
- Meanwhile the inspector shows: what to click; **Standard** (the fourteen standard stamps); **With your name and the
  time** (Approved, Reviewed, Received, Revised, Confidential); **Your stamps** (the library, or how to add one);
  **Import…**, and **Remove** when a library stamp is chosen.
- The click puts the chosen stamp there, centred, kept on the page, and opens its note in the inspector.

## Mode

Annotate (the command works in every mode).

## Speed and size impact

Nothing at startup. The library folder is read when the Stamp command turns on. A dynamic stamp is laid out by MuPDF
when placed (a few milliseconds); its art goes into the file and into the operation log as hex.

## Design

- **Model** (`annotate::stamps`, pure apart from reading files): `Standard`, `Dynamic`, `Fields::fill`,
  `dynamic_html` and `dynamic_art`, `place`, `fit`, `changes`, and the library (`library`, `import`, `remove`,
  `Item::art`).
- **Engine:** `Change::StampArt { page, id, art }` follows the stamp's `CreateAnnotation` in one operation. The shim's
  `ex_annot_set_stamp_art` sets a one-page PDF's display list as the appearance
  (`pdf_set_annot_appearance_from_display_list`) or an image (`pdf_set_annot_stamp_image`). `ex_html_to_pdf` lays
  HTML out with MuPDF's story engine; `ex_page_to_pdf` writes a page as a one-page PDF.
- **Names:** standard stamps use their standard name, drawn by every reader; the others a name that is not standard,
  so MuPDF keeps their appearance (DD18).
- **App:** `Placing::Stamp`; the picker in the inspector; the library folder is `stamps` in the app's data folder.

## Acceptance tests

- [x] `standard_stamps_are_the_standard_s_fourteen_names`, `a_stamp_is_centred_on_the_click_inside_the_page`.
- [x] Dynamic fields show the placement time: `dynamic_fields_are_filled_when_placed`,
  `a_dynamic_stamp_is_drawn_as_wide_as_set_and_as_tall_as_its_text`, `html_becomes_a_page_as_tall_as_its_text`.
- [x] One step of undo: `a_drawn_stamp_is_one_operation_of_two_changes`; the log keeps the art:
  `changes_read_back_as_they_were_written`.
- [x] Library: `library_names_are_safe_and_kept_apart`, `a_library_stamp_is_imported_listed_drawn_and_removed`.
- [x] The art is drawn, not the name, and kept when the stamp is moved: `a_stamp_is_drawn_as_its_art_and_keeps_it_when_moved`.
- [ ] Stamps look the same in Acrobat: not checked (no Acrobat here).
- [ ] Tried by hand: not yet.

## Open

- Times are UTC (DD18) until U.7 brings local formats.
- The dynamic stamps' wording is fixed; a template editor is not in the plan.
- Remove takes a library stamp away without asking (the file it was imported from stays).
