# Ph1: 21/60 | R.10: Recent and pinned files

**Tag:** R.10; **Priority:** P0; **Phase:** 1; **Needs:** K.20; **Code:** `excellent-store` (`recent`, `home`,
migration `0003_recent.sql`), `excellent-app::recent`, `excellent-app::tabs::list`
**Plan:** §27 entry `Ph1: 21/60 | R.10`.

## Problem

A reader comes back to the same few documents day after day. Finding each again in the file manager, and then the
page they were on, is slow.

## User story

As a reader, I want the files I opened lately, at the page I left them, one press away, with the ones I always need
pinned at the top, so that I can pick up where I stopped.

## UI sketch

- With no tab open, the document panel shows, under "Open a PDF", a "Pinned" section and a "Recent" section (with a
  "Clear" button that keeps pinned files). Each row: the file's name, its folder, and "Page *n*" (or "File not found");
  pressing it opens the file at that page, or shows its tab if it has one. Beside it, a pin toggle and a remove (×)
  button. Up to eight rows a section.
- The tab list (Ctrl+Shift+A) gains a "Recent files (*n*)" section with every recent file no tab has open; choosing
  one opens it at its page.
- Screen readers hear "Open *name* at page *n*", "Pin *name*" (a toggle, pressed when pinned), "Remove *name* from
  recent files", "Clear recent files (pinned files stay)".

## Mode

All modes. Not shown until a file has been opened once.

## Speed and size impact

- Startup: nothing. The list is read from the library the first time it is shown (the empty panel or the tab list).
- Showing a file writes one row on the library's writer thread, a second after the reader stops on a page and when
  the app quits; nothing waits on it.
- Portable copies run one pass over the library's file paths at startup, only when the app's folder changed since
  the last run.
- Installer: code and strings only.

## Design

- Storage (`0003_recent.sql`): `recent(file, seen, page, pinned)`, one row per file, and `meta(key, value)` for facts
  about the library itself. Pinned files stay until unpinned; of the others the newest 500 are kept.
- `excellent_store::recent`: `load` (pinned first, then newest), `visit`, `set_pinned`, `remove`, `clear`.
- Portable moves (`excellent_store::home`): the library notes the portable app's folder. When the app runs from a
  new folder, files inside the old folder get the same place inside the new one; files elsewhere on the old folder's
  drive move to the new drive letter, only when the letter changed, the file is gone from its old path and is at the
  new one.
- The window's copy (`excellent-app::recent`): lazily loaded, changed optimistically by the same rules as the library
  (tests keep the two in step), each change sent to the writer thread.

## Acceptance tests

From the entry's **Done when** ("the recent list survives restarts and portable-mode moves"):

- [x] `the_list_puts_pinned_files_first_then_the_newest`: visit, pin, remove and clear, saved and read back.
- [x] `only_the_newest_unpinned_files_are_kept`: the list's limit; pinned files stay.
- [x] `files_inside_the_portable_folder_move_with_it`,
  `files_beside_it_follow_a_new_drive_letter_only_when_found_there` (Windows): where a file is after a move.
- [x] `the_library_finds_its_files_after_the_app_moves`: the library's paths follow the app's folder.
- [x] `the_window_keeps_the_list_in_the_library_order`, `the_window_keeps_as_many_files_as_the_library`: the window's
  copy matches the library.
- [ ] Tried by hand: the empty panel, the tab list section, a portable copy moved to another folder.
- [ ] Thumbnails on the rows (step 1): they need a cover rendered for a file that isn't open (R.5 draws only the
  shown file's pages).
- [ ] The File menu, the Windows jump list and the macOS Dock menu (step 2): with the menu bar and with R.11's file
  associations.
- [ ] The home page (F.17): the empty document panel stands in for it until then.
