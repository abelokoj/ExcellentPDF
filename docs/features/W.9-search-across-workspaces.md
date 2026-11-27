# Ph3: 12/49 | W.9: Search tabs across workspaces

**Tag:** W.9; **Priority:** P1; **Phase:** 3; **Needs:** R.12; **Code:** `ecclesia-app::tabs::list`,
`ecclesia-app::workspaces`, `ecclesia-store::workspaces`
**Plan:** §27 entry `#### Ph3: 12/49 | W.9`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A reader with many workspaces remembers the paper, not where they put it. Finding it means switching from one
workspace to the next and looking.

## User story

As a reader with several workspaces, I want to type a file's name and land on its tab in whichever workspace has it,
so that I don't hunt workspace by workspace.

## UI sketch

- The tab list (Ctrl+Shift+A; the palette; the strip's list button) keeps its sections and adds one for each other
  workspace with tabs, between the closed tabs and the recent files: "In Tax (12)", or "In Tax, another window (12)".
- A row is the file's name; typing matches the name, the folder and the workspace's name, so "tax" lists the Tax
  workspace's tabs.
- Return on a row switches to its workspace and shows the tab, its pane the active one. When another window shows the
  workspace, that window comes to the front with the tab shown.

## Mode

All modes.

## Speed and size impact

One query when the list opens, over the saved tabs (indexed by workspace); the rows are made once. Filtering is the
NSH command list's, the same as for the shown workspace's tabs. No document opens until a row is picked.

## Design

- `ecclesia-store::workspaces::tabs_elsewhere`: every workspace's open tabs but the shown one's, in the list's order,
  then by pane and strip.
- `Workspaces::tabs_elsewhere` groups them under the shared workspace list; `Workspaces::show_tab` goes to a tab: the
  window that shows its workspace (W.8), or a switch here, then the pane that has the tab.
- `tabs::list::elsewhere_rows` is a pure function of those groups, unit-tested.

## Acceptance tests

- [x] `the_other_workspaces_tabs_are_listed_in_order`: the shown workspace is left out; the others come in the list's
  order.
- [x] `other_workspaces_tabs_are_listed_by_workspace`: one section per workspace, its name a search word, another
  window's marked.
- [ ] 5,000 tabs filter as you type within 16 ms.
- [ ] Tried by hand: a tab in another workspace, and in a workspace another window shows.

## Open

- Not tried by hand; the 16 ms budget for 5,000 tabs is not measured.
- Another window's own workspace is listed as last saved, which is up to a second behind its strip.
