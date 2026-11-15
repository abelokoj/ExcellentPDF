# Ph1: 23/60 | R.12: Tabs

**Tag:** R.12; **Priority:** P0; **Phase:** 1; **Needs:** K.5; **Code:** `excellent-app::tabs` (`model`,
`strip`, `list`), `excellent-app::shell`, `excellent-app::commands`, `excellent-app::settings_window`
**Plan:** §27 entry `Ph1: 23/60 | R.12`.

## Problem

The app showed one document, the one on the command line. Readers keep several open at once (a paper and its
references, a manual and its errata) and move between them all day.

## User story

As a reader, I want every document I open in a tab of its own, so that I can go back and forth between them with a
click or a key, close what I am done with, get back what I closed by mistake, and find any tab by its name.

## UI sketch

- Title bar, left: the tab strip, in NSH's tab list (one Tab stop; Left and Right move between tabs and show each).
  A tab: its file's name (cut short with an ellipsis), a dot while it has unsaved changes, a pin mark when pinned,
  and a close button (none on pinned tabs). The shown tab takes the page's background, as if the page came up into
  it. Tabs shrink as more open, to 56 points each. A middle click closes a tab. After the tabs, a + button opens
  files (Ctrl+O); at the right end of the strip, a chevron opens the tab list. With nothing open, the app's name
  stands in the strip's place.
- The empty document panel: "No document open", "Press Ctrl+O to open a document, or Ctrl+K to see every command",
  and an "Open a document" button.
- Open: the system's open dialog (NSH's `pick_paths`), several files at once, filtered to the formats MuPDF opens.
- Keys: Ctrl+O open, Ctrl+W (Ctrl+F4) close, Ctrl+Shift+T reopen the last closed tab where it was, Ctrl+Tab and
  Ctrl+Shift+Tab (Ctrl+PageDown, Ctrl+PageUp) next and previous, Ctrl+Shift+PageUp / PageDown move the tab,
  Ctrl+Shift+A the tab list. Pin, close others, close to the right and close duplicates are in the command palette.
- Tab list (Ctrl+Shift+A, a dialog like the palette: a dropdown under the title bar would be drawn behind the page
  canvas): a search field over counted sections, "Open (n)", "Open more than once (n)" and "Recently closed (n)".
  Typing searches names and folders; Return shows a tab, reopens a closed one, or (on a duplicate row) keeps one tab
  of that file; Delete closes the highlighted open tab.
- Settings window, Tabs, "Ctrl+Tab order": Recently used (the default) or Tab order (`tab_cycle_order`).

## Mode

All modes.

## Speed and size impact

- Startup: one tab for the command line's file; with none, no page canvas (and no swap chain) is made until a tab
  is shown.
- Only the shown tab has a document open: switching gives the one canvas the other file (it reopens through the
  shared registry, K.5), and the page the reader left that tab at comes back once the pages are reported. Not
  measured (K.24).
- Installer: code and strings only.

## Design

- Model (`tabs/model.rs`, pure Rust): `Tabs` holds the tabs in strip order (pinned ones first), the shown one, the
  order they were last shown in, and the last 25 closed (with their place and page). Open, activate, close (shows
  the tab shown before), close others or to the right (sparing pinned tabs), close duplicates, reopen, move within
  the tab's group, pin, the page each tab is at, Ctrl+Tab's next in recent or strip order, and the files open twice.
- `TabState` (`tabs/mod.rs`): the model in a signal, the open dialog, the page to bring back, and the commands'
  actions. The canvas's `path` property is the shown tab's file; the outline, the thumbnails and file links read the
  shown tab's file instead of the command line's.
- Commands: `file.open`, `tab.close`, `tab.reopen_closed`, `tab.next`, `tab.previous`, `tab.list`, `tab.move_left`,
  `tab.move_right`, `tab.pin`, `tab.close_others`, `tab.close_right`, `tab.close_duplicates`.

## Acceptance tests

From the entry's **Done when** ("50 tabs open with only visible ones loaded; the tab-list dropdown finds any of them
by name"):

- [x] `closing_shows_the_tab_shown_before_and_reopening_puts_it_back`,
  `close_others_and_to_the_right_spare_pinned_tabs`, `tabs_move_within_their_group`,
  `ctrl_tab_follows_recent_use_or_the_strip`, `duplicates_are_found_and_closed`,
  `a_closed_tab_is_reopened_by_its_path_and_the_list_is_bounded`: the model.
- [x] `only_window_commands_are_offered_without_a_document`: the tab commands need a tab; open and reopen don't.
- [ ] 50 tabs opened by hand, with only the shown one loaded (by design: one canvas), and each found in the tab list.
- [ ] Reordering by dragging a tab with the mouse (the keys move tabs meanwhile).
- [ ] A tab's context menu (pin, close others, close to the right): the commands are in the palette meanwhile; a
  popup menu would be drawn behind the page canvas, as the tab list would.
- [ ] Workspaces (W.1, W.4, W.9): per-workspace closed tabs, tab search across workspaces and its badge.
- [x] Files from Explorer and a second launch (R.11) opening as tabs: `a_launch_shows_its_files_in_their_tabs_or_new_ones`.
