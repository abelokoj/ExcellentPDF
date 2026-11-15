# Ph1: 8/60 | K.18: Modes and toolbars

**Tag:** K.18; **Priority:** P0; **Phase:** 1; **Needs:** K.16, K.17; **Code:** `excellent-features::modes` (model),
`excellent-app::modes` (view)
**Plan:** §27 entry `Ph1: 8/60 | K.18`, §10.2 (modes), §2.1 rule 3.

## Problem

Every tool on screen at once makes a reader hunt; the plan keeps the window clean by showing only what the reader is
doing now. Without modes there is no toolbar at all, so mouse readers reach commands only through the palette.

## User story

As a reader, I want a bar that shows the tools of what I'm doing (reading, marking up, presenting), set the way I
like, so the window stays quiet and the tools I use are one click away.

## UI sketch

- **Mode bar** (40 points, under the window bar; hidden in full screen): the mode switcher (Read, Annotate, Present;
  Write and Study join it once they have tools), a rule, then the mode's tools as short text buttons, then
  **Customize**. The chosen mode is filled with its accent (the `mode-*` tokens); the others carry a dot of theirs. The
  bar's bottom edge and the shown tab's top edge take the mode's accent, so the mode is visible at a glance.
- The switcher and the tools are one toolbar: one Tab stop, arrows move along it. Each tool is heard with its full
  label and shortcut ("Find in document, Ctrl+F"); the bar shows a short word ("Find") so a full toolbar fits a
  laptop. A narrow window scrolls the tools sideways rather than clipping them. A tool that needs a document is dimmed
  without one.
- **Customize toolbar** (a dialog, the list dialogs' look): the five modes as tabs; the mode's items, each dragged by
  its name or moved with ↑ ↓ and removed with ×; **Add a tool** with a search field over every command, and **Add
  separator**; **Reset** (this mode's defaults), **Export…**, **Import…**, **Done**. Escape closes it. A notice says
  what an export or import did, or why a file was refused (not a preset; from a newer version).
- **Keys:** Ctrl+Shift+M next mode, Ctrl+Alt+Shift+M previous mode (among those offered). Every mode, Customize, Export
  and Import are in the palette.
- **Settings, Modes and toolbars:** **Pen switches to Write** (on by default), **Toolbars: Customize…**.

## Mode

This is the modes feature. Default toolbars:

| Mode | Tools |
| --- | --- |
| Read | Zoom out, Zoom in, Fit width, Fit page; Single, Facing, Continuous; Find, Theme, Full screen |
| Annotate | Bookmark, Outline entry; Undo, Redo (the markup tools come with the A.* tags) |
| Write | none yet (H.* tags) |
| Study | none yet (S.* tags) |
| Present | Full screen; First page, Screen up, Screen down, Last page; Reading ruler, Auto-scroll |

## Speed and size impact

- Startup: the bar is built with the window (a few dozen views); the mode and the toolbars are read from the settings
  file already loaded. The customize dialog is built only while it is open. No timing measured.
- Installer: a few kilobytes of code and 60 strings per language.

## Design

- **Model** (`excellent-features::modes`, pure Rust): `Mode` (`id`, `from_id`, `step` among the modes offered),
  `after_pen_contact`, `Toolbar` (items by command name: `parse`, `to_text`, `add`, `add_separator`, `remove`,
  `move_item`; unknown names dropped, no duplicate command, no separator at an end or twice), `Preset` (JSON:
  `{"excellentpdf_toolbars": 1, "toolbars": {"read": ["view.zoom_in", "|", …]}}`, `to_json`, `from_json` with
  `PresetError::{NotJson, NotPreset, Newer}`; unknown commands are counted and left out).
- **View** (`excellent-app::modes`): `Modes` (the settings, the dialog's state), `mode_bar`, `customize_dialog`. The
  tab strip reads the mode from the context for its accent.
- **Pen:** the Windows canvas sends `penContact` on a pen's first touch; the pane hands it to `Modes::pen_contact`,
  which switches to Write when the setting is on and Write is offered. Ink still draws in any mode until H.1 gates it.
- **Commands:** `mode.read`, `mode.annotate`, `mode.write`, `mode.study`, `mode.present`, `mode.next`,
  `mode.previous`, `toolbar.customize`, `toolbar.export`, `toolbar.import`.
- **Settings:** `mode` (text, default Read), `toolbar_<mode>` (text, written only when it differs from the default),
  `pen_switches_to_write` (flag, default on). The mode is the app's, not a workspace's (DD8).
- **Files:** an export is written atomically (`settings::write_atomically`).

## Acceptance tests

- [x] `each_mode_shows_only_its_tools_and_the_toolbar_can_be_customized`: Read shows Read's tools and not Present's;
  Present the reverse; the next-mode key returns to Read; the customize dialog opens and Escape closes it.
- [x] `read_and_present_show_only_their_own_tools`, `default_toolbars_name_only_commands_that_exist`: the defaults.
- [x] `presets_round_trip_and_leave_out_what_this_version_lacks`, `files_that_are_not_presets_are_refused`: presets
  round-trip.
- [x] `toolbars_keep_known_commands_once_and_tidy_separators`, `edits_move_add_and_remove_items`,
  `modes_step_around_those_offered_and_the_pen_asks_for_write`.
- [ ] Not tried by hand: dragging rows, export and import through the file dialogs, a real pen.
