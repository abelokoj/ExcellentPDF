# Ph1: 20/60 | R.9: Session restore

**Tag:** R.9; **Priority:** P0; **Phase:** 1; **Needs:** K.5, K.20; **Code:** `excellent-app::session`,
`excellent-app::tabs` (`TabState::current`, `locate`), `excellent-app::shell` (`document_missing`)
**Plan:** §27 entry `Ph1: 20/60 | R.9`, §11.16.

## Problem

Quitting the app lost every tab: the reader had to find and open each document again and scroll back to their page.

## User story

As a reader, I want the app to come back exactly as I left it, every tab at its page, so that I can quit or restart
without losing my place.

## UI sketch

- Nothing to do: the tabs are saved a second after they change and when the app quits, and come back when it starts,
  the shown one shown, each at the page it was left at. A file given on the command line opens in its own tab (or
  shows the session's tab of it).
- A tab whose file has gone shows a hollow ring and a greyed name in the strip ("File not found" to screen readers).
  Shown, its panel says "*name* wasn't found", "It was in *folder*. It may have been moved, renamed or deleted." and
  offers "Locate…", which opens the system's dialog and shows the file from where it is now.
- Recently closed tabs come back too (Ctrl+Shift+T works across restarts).

## Mode

All modes.

## Speed and size impact

- Startup: one read of the library and one `stat` per tab, so missing files show at once. Only the shown tab's
  document opens (W.14); the others are rows until shown. Not measured (K.24).
- Saving runs on the library's writer thread, debounced by a second; quitting waits for the last save.
- Installer: code and strings only.

## Design

- `session::restored()` reads the default workspace's tabs (K.20) and turns them into the tab model, marking the
  tabs whose files are missing. `session::keep(state)` watches the tabs and the shown page, and saves a second after
  the last change and on quit (`on_app_quit`, then `flush`).
- A tab's View state is the page canvas's `view` event (`excellent-canvas-win::saved`): the page and the place on it
  (as fractions of the page), the zoom and how it was chosen, the layout, paged or continuous, the direction and the
  view rotation. The canvas reports it when the view settles somewhere new; the tab keeps it while it is at the
  view's page, and the session saves it in place of `{"page": 3}`, which older sessions and a page given alone (a
  launch's) still use. Shown again, the tab's file opens and `setView` puts it all back. The page box isn't kept (each
  document opens on its crop box); a grid's columns follow the window. A book (R.28) goes back to its page at the
  zoom it is laid out for.
- The tab model gains `missing`, `restore`, `relocate` and `remember_closed`; `TabState::current()` is the tabs with
  the shown one at the page it shows.
- Windows, panes and workspaces are saved as the one default workspace until W.1 and W.4 bring more.

## Acceptance tests

From the entry's **Done when** ("quitting and restarting restores every tab at the same place"):

- [x] `a_session_comes_back_with_every_tab_at_its_page`: tabs, pins, the shown tab, closed tabs and groups survive a
  save and a load.
- [x] `a_tab_whose_file_has_gone_comes_back_missing`: missing files are marked.
- [x] `a_view_state_without_a_page_starts_at_the_first`: broken or foreign View states start at page 1.
- [x] `tabs_come_back_as_they_were_saved` (K.20): the database side.
- [ ] Quit and restart by hand with several tabs, a missing file and "Locate…".
- [x] `a_tab_keeps_its_view_while_it_is_at_the_view_page`, `a_saved_view_comes_back_as_it_was`,
  `a_page_alone_or_a_broken_view_is_not_a_view`: zoom, layout, rotation and the position within the page are saved
  with the tab and come back; a view left at another page gives way to the page.
- [ ] The order Ctrl+Tab goes through recently used tabs (it starts as the strip's).
- [ ] Several windows, panes and workspaces (W.1, W.4).
