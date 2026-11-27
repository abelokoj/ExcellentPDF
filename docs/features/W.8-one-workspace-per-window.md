# Ph3: 11/49 | W.8: One workspace per window

**Tag:** W.8; **Priority:** P1; **Phase:** 3; **Needs:** W.7; **Code:** `ecclesia-app::workspaces` (`claims`,
`windows`), `ecclesia-app::shell`, `ecclesia-store::workspaces`
**Plan:** §27 entry `#### Ph3: 11/49 | W.8`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

One window shows one workspace. A reader with two screens wants the thesis on one and the tax papers on the other,
each with its own tabs, panes and sidebar.

## User story

As a reader with two screens, I want a workspace in a window of its own, so that I can work in two workspaces side
by side.

## UI sketch

- "Open workspace in new window" (Ctrl+Alt+Shift+N; the palette; the workspace list's actions) lists the other
  workspaces. The one picked opens in a new window with the main window's whole shell: title bar, tabs, panes, the
  sidebar, the inspector, the palette, the dialogs and the modes.
- The workspace list marks a workspace another window shows ("open in another window"). Picking it there, or its
  Ctrl+Alt+number, brings that window to the front.
- The window's title is its workspace's name.

## Mode

All modes.

## Speed and size impact

Nothing until a window opens. Each workspace window is one more shell, its panes holding only their shown tabs'
documents (W.14). The list of workspaces is one signal all windows share.

## Design

- The rule is a pure model, `workspaces::claims::Claims`: which window shows which workspace. Every way a window
  comes to show a workspace asks it: the start, a switch (the list, Ctrl+Alt+1 to 9), a new workspace, opening one in
  a window, a file another workspace has (W.6), a deleted workspace's neighbour, the next or previous one (W.5).
  When another window has it, that window is the answer: it comes to the front (`WindowRef::focus`) and nothing
  changes here.
- `workspaces::windows` is the registry on the UI thread: each window's number, `WindowRef` and `Workspaces`. A file
  whose workspace another window shows opens in that window; a tab sent there is adopted at once rather than
  written to its saved tabs, which its window would write over.
- The shell is split: `shell()` is the app's part, done once (theme, language, contrast, the windows of the last
  run) and the main window's; `open_workspace_window` makes a window's panes and workspaces under a root of their
  own, opens it with `open_window`, and disposes them when its content goes. `window_shell` is every window's.
- Closing a workspace window docks its popped-out tabs (V.11), saves its edits (K.15) and tabs, and frees its
  workspace. The main window keeps its workspace in `meta` (W.7); the others' are a list under
  `workspace.windows`, which the next start opens again.
- Full screen (R.19) keeps its own window's ref: commands run outside any window, where `current_window` is the main
  one.

## Acceptance tests

- [x] `a_workspace_shows_in_one_window_at_a_time`: a second window can't claim a shown workspace and is told which
  window has it; a switch gives up the old one; a closed window frees its own; the free ones for cycling.
- [x] `the_other_windows_workspaces_are_remembered_until_they_go`: the store keeps the other windows' workspaces in
  order and drops deleted ones.
- [ ] Tried by hand: two windows, each its own workspace; every open path focuses the window that has a workspace;
  quitting and starting again brings both back.

## Open

- Not tried by hand.
- Whether quitting with workspace windows open keeps them for the next start depends on NSH not reporting their
  closing at quit; not checked.
- Closing the main window while workspace windows are open: what the app does then is NSH's (it ends with its last
  window); not checked.
