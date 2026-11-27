# Ph5: 9/45 | B.13: Custom outline for non-PDF formats

**Tag:** B.13; **Priority:** P2; **Phase:** 5; **Needs:** B.6; **Code:** `ecclesia-features::outline_extra::sidecar`
**Plan:** §27 entry `#### Ph5: 9/45 | B.13: Custom outline for non-PDF formats`.

## Problem

CBZ, DjVu, and many EPUB/MOBI files either lack an internal table of contents or are read-only containers that cannot store edited PDF bookmarks.

## User story

As a reader of a CBZ comic or EPUB book, I want to create and edit a custom outline stored in an atomic sidecar so my chapters survive app restarts.

## UI sketch

- Outline sidebar enables Add / Rename / Delete / Reorder for non-PDF formats when backed by `SidecarOutlineStore`.
- `effective_outline(sidecar, builtin)` shows the custom sidecar outline whenever present, with a "Revert to built-in outline" action (`SidecarOutlineStore::clear`).

## Mode

All modes when viewing a non-PDF format.

## Speed and size impact

- Startup: zero cost; loaded lazily for the active non-PDF document.
- Storage: atomic temp-file + `sync_all` + rename writes keyed by SHA-256 of document path.

## Design

- **Model (`ecclesia-features::outline_extra::sidecar`):**
  - `SidecarOutlineStore` persisting `Entry` trees via lossless JSON (`outline_extra::io`).
  - `effective_outline` selecting sidecar entries over built-in ones when present.

## Acceptance tests

- [x] `outline_extra::sidecar::tests::custom_cbz_outline_persists_across_store_reopen`: verifies a custom CBZ outline survives store reopen (app restart) and clears cleanly.
