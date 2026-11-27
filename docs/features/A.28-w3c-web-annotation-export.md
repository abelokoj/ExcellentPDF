# Ph3: 38/49 | A.28: W3C Web Annotation export

**Tag:** A.28; **Priority:** P1; **Phase:** 3; **Needs:** K.28; **Code:** `ecclesia-features::annotate::w3c`,
`ecclesia-app::annotate::exchange` (the W3C format beside A.24's XFDF and FDF)
**Plan:** §27 entry `#### Ph3: 38/49 | A.28`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Web annotation tools (Hypothesis, Recogito, research note apps) share annotations as W3C Web Annotations, not XFDF.
Without that format, highlights and notes made here can't travel to them, and theirs can't come here.

## User story

As a researcher, I want to export my highlights and comments as W3C Web Annotations, and import such a file, so they
move between this app and the web tools I also use.

## UI sketch

- **Export** (Annotate toolbar, **Ctrl+Alt+E**, palette "Export annotations (XFDF, FDF, W3C)…"): the save dialog
  offers "W3C Web Annotations (.jsonld)" beside XFDF and FDF.
- **Import** (**Ctrl+Alt+I**): the open dialog takes `.jsonld` and `.json` too. The summary dialog counts what was
  added, updated, unchanged and skipped, as for XFDF.

## Mode

Annotate and Review.

## Speed and size impact

Nothing at startup. Files are written and read on the workers; placing another tool's quotes reads the document's
text at background priority, as A.16's transfer does.

## Design

- **Model (`w3c`):** `write` makes an `AnnotationCollection` using only the terms of the W3C context
  (`http://www.w3.org/ns/anno.jsonld`): each annotation has its `urn:uuid` id, motivation (highlighting, commenting
  or replying), creator, dates, text bodies (comment, tags as `tagging`, review state as `assessing`) and a target of
  the PDF with a `FragmentSelector` (`page=N&viewrect=…`, RFC 3778), a `TextQuoteSelector` and a
  `TextPositionSelector` when K.28 anchored it. A reply targets its parent's id. Each annotation also carries its
  whole own record under the full IRI `urn:ecclesiapdf:annotation`, which JSON-LD processors keep as is and other
  tools ignore.
- **Import:** `read` returns our own annotations (from that record) and other tools' quotes. Ours merge by id as
  XFDF does (A.24). A quote becomes a highlight that A.16's transfer places by finding its text; one whose text isn't
  found is skipped.

## Acceptance tests

- [x] Exported JSON uses only terms the model's context defines: `the_export_uses_only_the_context_s_terms`.
- [x] Our export comes back whole: `our_export_comes_back_whole`.
- [x] Another tool's text quote becomes a highlight to place: `another_tool_s_quote_becomes_a_highlight_to_place`.
- [x] Dates: `iso_dates_go_both_ways`; the file's format: `the_format_is_told_by_extension_or_header`.
- [ ] Validated by a JSON-LD processor, and tried with Hypothesis by hand: not yet.

## Open

- Validation checks the terms against the context's list; no JSON-LD processor runs in the tests (DD23).
- Shapes other than text markup export with their page region only; other tools' non-text targets (SVG selectors,
  images) are skipped.
