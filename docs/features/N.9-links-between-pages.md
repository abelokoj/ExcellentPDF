# Ph4: 22/44 | N.9: Links between pages

**Tag:** N.9; **Priority:** P1; **Phase:** 4; **Needs:** R.7; **Code:** `ecclesia-features::notebook::links`
**Plan:** §27 entry `#### Ph4: 22/44 | N.9`.

## Problem

Long notebooks and multi-notebook study sets benefit from clickable cross-references between index pages, topic pages, and external reference PDFs.

## User story

As a student building a study index, I want to link a region on one page to another page or another notebook so links continue working in exported PDFs.

## UI sketch

- Selection/region context menu: "Link to page..." (`notebook.link_to_page`) or "Link to document...".

## Mode

Annotate and Write modes (clickable in all modes via R.7).

## Speed and size impact

- Startup: zero cost; links are standard PDF `/Link` annotations.

## Design

- `add_internal_link` writes a PDF `/Link` annotation with `/Dest [page_ref /XYZ ...]` and stores metadata in `notebook.json` (kept updated on N.2 page insertions and N.8 section reorders).
- `DeepLink` builds and parses `ecclesiapdf://doc/<id>?page=<p>&x=<x>&y=<y>` URIs (F.13) written via `/A << /S /URI ... >>`.

## Acceptance tests

- [x] `internal_and_deep_links_work_in_exported_pdfs`: adds both an internal page link and an `ecclesiapdf://` deep link, exports plain PDF, and verifies MuPDF's `page.links()` resolves both the internal destination page and the deep-link URI.
