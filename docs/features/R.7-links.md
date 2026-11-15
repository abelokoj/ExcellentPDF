# Ph1: 18/60 | R.7: Links

**Tag:** R.7; **Priority:** P0; **Phase:** 1; **Needs:** K.8; **Code:** `excellent-features::read::links`,
`excellent-mupdf::link`, `excellent-canvas-win::links`, `excellent-app::links`
**Plan:** §27 entry `Ph1: 18/60 | R.7`, §14.2.

## Problem

Links in a document did nothing, and a link that starts a program or runs a script must never do so silently.

## User story

As a reader, I want to click a cross-reference or a web link and land where it points, so that I can follow the
document; and I want to be asked before a link opens a file or another app.

## UI sketch

- Over a link the cursor is a hand, and the title bar shows where it goes: "Go to page iv (4 of 210)" or its address.
- A click on a link inside the document jumps; Back (Alt+←) returns.
- A web or mail link opens in its app at once. Any other protocol, a file or a launch action asks first, in the app's
  own dialog: the address, Cancel (focused first) and Open. A script link shows that it never runs, with OK.

## Mode

Every reading mode.

## Speed and size impact

Links are read on the render workers for the pages on and near the screen (8 pages either way), kept while their page
stays near, and dropped with it. Nothing is read at startup. Not measured.

## Design

- `excellent-mupdf`: the shim walks `fz_load_links` and resolves internal links with `fz_resolve_link`; `Link` holds
  the rect in page points, the address, and the target page and point.
- `excellent-canvas-win`: `Links` caches each near page's links; the canvas maps the mouse to a page point through
  `Placement`, shows the hand, emits `linkHover` and, for a link out of the document, `openLink`. An internal link
  jumps through the same path as go to page, so it pushes history.
- `excellent-features::read::links::follow`: the policy. `javascript`, `vbscript`, `data`, `blob` and `about` are
  refused; `file:`, paths and drive letters ask; protocols on the reader's list open; any other asks.
- `excellent-app::links`: the hint, the dialog, and opening with NSH's `open_url` (the app declares the `openUrl`
  capability) or `open_path` for files, relative paths taken from the document's folder.
- Setting `link_protocols` (default `http,https,mailto`).

## Acceptance tests

- [x] `links_are_read_and_resolved`: a PDF with an internal, a web, a launch and a JavaScript link.
- [x] `web_links_open_and_other_protocols_ask`, `scripts_never_run_and_files_always_ask`.
- [x] `a_link_target_shows_at_the_top`, `file_links_become_paths`, `escapes_decode_and_stray_percents_stay`.
- [x] `epub_and_xps_links_are_read_and_resolved`: an EPUB's link to a chapter goes to its page, and web links in
  EPUB and XPS are read like a PDF's.
- [ ] Hover shows a page preview for internal links: needs thumbnails (R.5).
- [x] `link_protocols` in the settings UI: Settings, Links lists the protocols that open without asking.
