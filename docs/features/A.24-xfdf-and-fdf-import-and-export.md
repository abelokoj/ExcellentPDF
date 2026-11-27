# Ph3: 36/49 | A.24: XFDF and FDF import and export

**Tag:** A.24; **Priority:** P1; **Phase:** 3; **Needs:** K.14; **Code:** `ecclesia-features::annotate::xfdf`,
`ecclesia-app::annotate::exchange`, `ecclesia-core::annot::fdf_annotations`, `ecclesia-mupdf` (`page_transform`,
`to_fdf`, `with_fdf_annotations`), the shim's `ex_page_transform`, `ex_fdf_write` and `ex_fdf_read`
**Plan:** §27 entry `#### Ph3: 36/49 | A.24`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reviewers who use other PDF tools send their comments as XFDF or FDF files, and expect the same back. Without import
and export, comments can only travel inside the PDF, so each round means swapping whole documents.

## User story

As a reviewer, I want to export my annotations as an XFDF or FDF file that Acrobat imports, and import such a file
here so that annotations already in the document are updated, new ones added, and nothing is doubled.

## UI sketch

- **Export** (Annotate toolbar, **Ctrl+Alt+E**, palette "Export annotations as XFDF or FDF…"): a save dialog named
  after the document (`report.xfdf`), offering XFDF and FDF. A dialog then says how many were written and where.
- **Import** (Annotate toolbar, **Ctrl+Alt+I**, palette "Import annotations from XFDF or FDF…"): an open dialog for
  `.xfdf` and `.fdf` files. The annotations are merged as one step of undo, then a dialog says how many were added,
  updated, already here and left out.

## Mode

Annotate and Review.

## Speed and size impact

Nothing at startup. Export and import read the document's annotations on the render workers, as the annotation list
does, and parse or write the file on a blocking thread; the document's lock is held only while pages are read. One
small dependency (`quick-xml`, MIT).

## Design

- **Model** (`annotate::xfdf`, pure): `write` turns annotations into XFDF (ISO 19444-1) through `AnnotData`, the flat
  form MuPDF reads and writes, with positions mapped from page space to PDF space by each page's `PageSpace`; `read`
  parses XFDF back (Acrobat's rich text read as plain text; dates with any time zone); `merge` finds the changes by
  id: an annotation already here is updated (deleted and created when it moved page), a new one added, a locked one
  or one on a missing page left out, one that is the same to a hundredth of a point counted and left alone. Creates
  come before the replies and group members that name them. What only this app keeps (tags, anchor, category,
  layer) rides in attributes of our own namespace (`urn:ecclesiapdf:xfdf`), which other readers pass over.
- **Engine:** the shim gives each page's PDF-to-page map, writes the PDF's annotations into an FDF file (each with
  `/Page`, replies' `/IRT` pointing at the copies), and reads an FDF's annotations onto a blank copy of the
  document's pages, so they are read with the same code as the document's own.
- **App:** `annotate::exchange`: the dialogs, the work on the workers, one `Operation` for the import, and the
  summary.

## Acceptance tests

- [x] Every kind the model keeps goes out as XFDF and comes back the same:
  `every_kind_comes_back_as_it_went_out`.
- [x] Positions are in PDF space, quads in Acrobat's order: `positions_are_written_in_pdf_space`.
- [x] Acrobat-style XFDF (rich text, time zones, popups, kinds we don't keep, pages we lack) reads:
  `acrobat_s_xfdf_reads`.
- [x] Dates both ways: `dates_go_both_ways`.
- [x] Merging by id: `importing_merges_by_id`.
- [x] FDF out and back with ids, pages, kinds and threads: `annotations_go_out_and_back_as_fdf`.
- [x] The format by extension or header: `the_format_is_told_by_extension_or_header`.
- [ ] XFDF exported here imports into Acrobat and back without loss: not checked (no Acrobat here).
- [ ] Tried by hand: not yet.

## Open

- XFDF has no place for a note's open state or a custom stamp's art; both are lost on the way through XFDF (FDF
  keeps them). See DD21.
- Tags, anchors, categories and layers survive only through tools that keep unknown attributes.
- Dates are written in UTC.
