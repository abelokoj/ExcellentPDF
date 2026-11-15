# Ph1: 24/60 | R.13: Named tab groups

**Tag:** R.13; **Priority:** P1; **Phase:** 1; **Needs:** R.12; **Code:** `excellent-app::tabs` (`groups`,
`group_dialog`, `strip`), `excellent-app::session`, `excellent-store` (migration `0002_tab_groups.sql`)
**Plan:** §27 entry `Ph1: 24/60 | R.13`; the idea of named sets of tabs is Enhanced's `src/TabGroupsManage.cpp`.

## Problem

With many tabs open, a reader working on two things at once (a thesis and a review) can't tell them apart in the
strip or put one set out of the way without closing it.

## User story

As a reader with many documents open, I want to gather tabs into named, coloured groups that I can collapse, so that
I can keep each project together and hide what I'm not working on without losing my place or my changes.

## UI sketch

- In the strip, a group is a rounded chip in its colour before its tabs, which are underlined in the same colour.
  The chip shows the group's name (a dot when it has none) and, while collapsed, the number of its tabs. Pressing it
  (or Enter and Space on it) collapses or expands the group; a right click opens its edit dialog. Screen readers hear
  "*name* tab group, tabs: *n*, expanded" (the colour stands in for a missing name; NSH v0.5.0's `Button`
  has no expanded state, so the label says it).
- Command palette: "Add tab to new group" (puts the shown tab in a new group and opens the edit dialog), "Edit tab
  group", "Remove tab from group", "Collapse or expand tab group".
- Edit dialog (a card near the top, like the go-to box): "Group name" field (focused; renames as you type; Enter
  closes), eight colour swatches named for screen readers (Blue, Red, Yellow, Green, Pink, Purple, Cyan, Grey; the
  chosen one ringed), then "Ungroup" and "Close group" (in the destructive colour). Escape or a press outside
  closes it.
- Moving a tab with Ctrl+Shift+PageUp / PageDown: beside an open group it joins it; at its group's edge it leaves;
  beside a collapsed group it passes the whole group.
- Collapsing the group of the shown tab shows the tab shown last outside it; showing a hidden tab (from the tab list
  or Ctrl+Tab's order, which skips hidden tabs) expands its group.

## Mode

All modes. Not visible until a group is made.

## Speed and size impact

- Startup: nothing more than R.9's load (groups are rows of the same read).
- Collapsing hides tabs from the strip only: no document is opened or closed.
- Installer: code and strings only.

## Design

- Model (`tabs/groups.rs`, pure Rust): `Group { id, name, color, collapsed }` and `GroupColor` (eight named colours,
  saved by name; an unknown name reads as grey). The rules: a group's tabs sit together in the strip; pinned tabs
  belong to no group; a group with no tabs is gone. New groups take the first colour no group has.
- Chip colours are fixed in every theme (Google's tab-group palette): each fill has a contrast of at least 4.5:1
  with its text (white, or near-black on yellow). The name is always shown or spoken, so the colour is never the
  only cue.
- Storage: `tab_group` rows per workspace (name, colour, collapsed, position) and `workspace_tab.tab_group`
  (`0002_tab_groups.sql`). Closed tabs keep no group; a reopened tab joins the group of the tabs on both sides.
- Commands: `tab.group.new`, `tab.group.edit`, `tab.group.leave`, `tab.group.toggle` (no default keys).

## Acceptance tests

From the entry's **Done when** ("groups survive restart; collapsing a group hides its tabs without unloading unsaved
work"):

- [x] `a_session_comes_back_with_every_tab_at_its_page`: a named, coloured, collapsed group survives a save and a
  load; `tabs_come_back_as_they_were_saved` (K.20): the database side.
- [x] `a_collapsed_group_hides_its_tabs_but_keeps_their_work`: a collapsed group's tab keeps its unsaved changes and
  page.
- [x] `a_group_keeps_its_tabs_together_as_tabs_move_in_and_out`, `the_only_group_with_the_shown_tab_does_not_collapse`,
  `a_group_goes_with_its_last_tab_and_reopened_tabs_rejoin_it`,
  `pinning_takes_a_tab_out_of_its_group_and_new_groups_get_new_colours`: the model's rules.
- [x] `a_group_shows_its_chip_before_its_tabs_and_hides_them_collapsed`: the strip's items.
- [x] `every_colour_has_an_english_name`: every swatch has a name.
- [ ] Tried by hand: the chip, the dialog, collapse with the shown tab inside.
- [ ] Dragging tabs into and out of groups with the mouse (with R.12's drag).
- [ ] Saving and reopening named sets of tabs as Enhanced does (Enhanced's "tab groups" are saved sets, not strip
  groups): with workspaces (W.1).
