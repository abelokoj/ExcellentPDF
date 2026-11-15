# Ph3: 13/49 | W.11: Export and import

**Tag:** W.11; **Priority:** P1; **Phase:** 3; **Needs:** W.1; **Code:** `excellent-store::workspace_file`,
`excellent-app::workspaces::file`
**Plan:** §27 entry `#### Ph3: 13/49 | W.11`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A workspace lives in the app's library. A reader can't keep a copy of it, hand a course's reading set to a
classmate, or take it to another computer.

## User story

As a reader, I want to save a workspace as a file and open it elsewhere, so that a set of papers and how I read them
goes where I go.

## UI sketch

- "Export workspace…" (the palette; the workspace list's actions) asks where to save `<name>.json` and saves the
  shown workspace there. A notice says where it went.
- "Import workspace…" asks for a workspace file and shows it as a new workspace, at the end of the list.
- Tabs whose files aren't where the file says are greyed out in the strip; showing one offers "Locate…" (R.9). The
  notice says how many there are.
- Errors are notices in plain words: not a workspace file, from a newer version, the folder can't be written to.

## Mode

All modes.

## Speed and size impact

Nothing until the reader asks. A workspace file is a few kilobytes; reading one is refused past 8 MB.

## Design

- The format is `excellent-store::workspace_file`, a pure module: `to_json(look, panes, root)` and
  `from_json(json, root)`. The file says what it is (`"kind": "excellentpdf-workspace"`) and its version; a newer
  version is refused.
- The plan's "chosen root" is the folder the workspace file is saved in, so picking where to save is choosing it.
  Paths under it are relative, with `/`; others are written as they are. Reading resolves relative paths against the
  folder the file is read from, so the file and its papers move together.
- Closed tabs stay behind: they are history, not the workspace.
- The file is untrusted: tab, group and active places past the end of their lists are dropped; panes past the app's
  four cells are dropped.
- Export writes the panes as they are now (`Keeper::current`), not as last saved; the write is atomic.

## Acceptance tests

- [x] `export_then_import_round_trips_exactly`: look, layout, panes, tabs, views, pins and groups come back the
  same.
- [x] `the_file_moves_with_its_papers`: relative paths follow the file's folder; others stay.
- [x] `a_file_that_is_not_a_workspace_is_refused`, `places_past_the_end_are_dropped`.
- [x] `a_workspace_name_makes_a_file_name`.
- [ ] Tried by hand: export, move the folder, import; a missing file greyed out and located.

## Open

- Not tried by hand.
- Paths are compared to the root as written (`Path::strip_prefix`); a different letter case on Windows makes a path
  absolute rather than relative.
