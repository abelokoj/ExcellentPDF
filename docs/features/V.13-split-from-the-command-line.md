# Ph1: 44/60 | V.13: Split from the command line

**Tag:** V.13; **Priority:** P1; **Phase:** 1; **Needs:** V.2, R.11; **Code:** `ecclesia-app::open`, `ecclesia-app::panes`
**Plan:** §27 entry `Ph1: 44/60 | V.13` and §11.15.

## Problem

Scripts and other apps (a LaTeX editor, a file manager) could not open two files side by side.

## User story

As a power user, I want `ecclesiapdf --split a.pdf b.pdf` to open both side by side.

## UI sketch

- `ecclesiapdf --split a.pdf b.pdf`: each file in a pane of its own (up to four; the rest are tabs of the last).
- `ecclesiapdf --split-same a.pdf`: the file in two panes.
- Both work at startup and when the app is already running (the launch crosses the single-instance channel).

## Mode

All modes; the split view appears only when the reader splits.

## Speed and size impact

- Startup: one pane, as before; the other cells' canvases are built only when they open.
- Budgets: each open pane is one more page canvas and its tiles (memory not measured, K.24). Nothing to the installer.

## Design

- `open::Split { Files, Same }` in `Launch`; `panes::route` places a cold start's files, `Panes::launch` a
  later launch's.

## Acceptance tests

- [x] `the_command_line_gives_absolute_files_and_a_page`: `--split` and `--split-same` are read
- [x] `a_launch_crosses_the_channel_unchanged`: the split crosses the single-instance channel
- [x] `split_puts_each_file_in_a_pane_of_its_own`: a split launch gives each file a pane
- [x] `split_same_shows_the_file_in_two_panes`: a split-same launch shows the file twice
