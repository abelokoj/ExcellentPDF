# Ph1: 54/60 | F.17: Home page

**Tag:** F.17; **Priority:** P0; **Phase:** 1; **Needs:** R.10; **Code:** `ecclesia-features::home`,
`ecclesia-app::home`, `ecclesia-app::recent`
**Plan:** §27 entry `Ph1: 54/60 | F.17`.

## Problem

A pane with no file open said only "No document open". A reader starting the app wants to get back to what they
were reading, or find a file they read before, in one press.

## User story

As a reader opening the app, I want to see what I was reading and my files with their covers, so I can carry on
at once.

## UI sketch

What every pane with no tab shows, top to bottom, in a column up to 640 points wide:

- **Home**, with the line on how to start (Ctrl+O, Ctrl+K) and **Open a document**.
- **Search recent files** (shown once there are recent files): keeps the pinned and recent files whose name or folder
  has every word typed, up to 50 of each; "No recent file matches" otherwise, pointing to the tab list.
- **Continue reading**: the file shown last that is still on disk, as a card with its cover, name, folder and page;
  pressing it opens the file at that page.
- **Workspaces** (with more than one): a chip per workspace with its colour, icon, name and tab count, the shown one
  ringed; pressing one switches to it (W.1).
- **Pinned** and **Recent** (R.10): each row now starts with the file's cover, then name, folder and page, Pin and
  Remove; Recent has Clear.

Covers are white paper until rendered (the file's type on it if it can't be). Every control has an accessible name;
the page scrolls.

## Mode

All modes; it is what an empty pane shows.

## Speed and size impact

- Startup: the recent list (one query, as before) and one `stat` per listed file for its cover's cache name; covers
  render later, so the page paints without waiting for them. Whether it paints within the cold-start budget is
  **not measured** (K.24).
- A cover not cached opens its file on a background thread (its own document, not the registry's, so a tab opening
  meanwhile never waits on it) and renders its first page on the render workers at `Priority::Thumbnail`, then keeps
  it as a PNG in the app's data folder (`covers/`), named by path, size and date.
- Nothing to the installer.

## Design

- Model (`ecclesia-features::home`): `continue_reading` (newest existing file, stat-ing only until it finds one),
  `matches` (every word, case ignored, in name or folder), `cover_name` (FNV-1a of path, size and date: the same
  name on every run).
- View (`ecclesia-app::home`): `Covers` (per window: rendered covers, files asked for, renders under way),
  `cover`, `home_page`; `recent::recent_panel` takes the search query and covers.
- Not built: **daily review** (S.5, a later phase; the page gets it with S.5); covers of pages other than the first;
  trimming the cover folder (K.20's housekeeping, as the thumbnails').

## Acceptance tests

- [x] `continue_reading_offers_the_file_shown_last_that_is_still_there`
- [x] `the_search_keeps_files_with_every_word_in_their_name_or_folder`
- [x] `a_cover_is_named_by_its_file_and_changes_when_the_file_does`
- [ ] the home page paints within the cold-start budget: **not measured** (K.24)
