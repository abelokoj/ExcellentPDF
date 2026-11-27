# Ph3: 18/49 | A.5: Sticky notes

**Tag:** A.5; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `ecclesia-features::annotate::sticky`,
`ecclesia-core::annot` (`icon`, `open`), `ecclesia-mupdf-sys/csrc/shim.c`, `ecclesia-app::annotate`
**Plan:** §27 entry `#### Ph3: 18/49 | A.5`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A note about a figure, a margin or a blank space has no text to highlight. Readers want a note pinned to a point, in
the icon every PDF reader shows.

## User story

As a reader, I want to put a sticky note anywhere on a page, choose its icon and whether it shows open, so that my
comments are where they belong and look the same in Acrobat.

## UI sketch

- `annotate.sticky` (palette, Annotate toolbar, `Shift+S`): the inspector says "Click where the note goes"; the next
  click on a page puts the note there and opens it. The command again calls it off.
- The inspector, for a sticky note: "Sticky note on page 2"; Icon (seven segments that wrap); "Open in other
  readers" (a switch); the note; Delete note and Done.

## Mode

All modes; the toolbar button is in Annotate mode's toolbar.

## Speed and size impact

None at startup. Placing reads the clicked page's size once.

## Design

- **Model** (`annotate::sticky`, pure): `Icon` (the standard's seven, `Note` by default), `place`, `set_icon`,
  `set_open`, `is_sticky`.
- **Core**: `Annotation::icon` and `Annotation::open`, through the log's JSON (older logs read as none and closed).
- **Shim**: `EX_ANNOT_ICON` reads the file's own `/Name`; the props' `is_open`; writing sets both for the kinds that
  have them.
- **App**: `AnnotateState::place_sticky` arms placing; the next `pagePress` places the note; the editor's sticky
  controls change it.

## Acceptance tests

- [x] `a_sticky_note_keeps_its_icon_and_open_state_through_save_and_reopen` (`/Name/Help` and `/Open true` in the
  bytes, read back).
- [x] `random_annotation_edits_survive_save_and_reopen` (now with icons and open states).
- [x] `a_note_is_placed_at_the_click_inside_the_page`, `icons_are_the_standard_s_seven_and_read_back_by_name`.
- [x] `changing_the_icon_or_open_state_is_one_change_only_when_it_changes`.
- [ ] Icons and open state match in Acrobat: not yet checked by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- Nothing on the page shows that a note is being placed; the inspector does.
- A sticky note can't be moved yet (A.13).
