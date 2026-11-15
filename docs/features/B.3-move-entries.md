# Ph1: 33/60 | B.3: Move entries

**Tag:** B.3; **Priority:** P0; **Phase:** 1; **Needs:** B.1; **Code:** as B.1
**Plan:** §11.14 step 2, §27 entry `Ph1: 33/60 | B.3`.

## Problem

An entry in the wrong place or at the wrong level could only be deleted and made again.

## User story

As a reader, I want to move entries up, down, in and out with the keyboard, so that the outline has the shape of
the document.

## UI sketch

- While the outline has the keyboard: Alt+Up and Alt+Down move the selected (or marked) entries one place; Tab
  indents them under the entry above; Shift+Tab outdents them after their parent. Escape gives the keyboard back to
  the pages (Tab no longer leaves the tree).
- The same four in the row's context menu and the palette ("Move outline entry up", and so on).
- Entries move with everything under them, and stay selected. A move that can't happen (the first entry up) does
  nothing.

## Mode

Every reading mode, PDF files.

## Speed and size impact

None at startup. A move is read subtree, delete, insert again, in one operation.

## Design

- `plan_shift` simulates the move on a copy of the tree and returns the steps (from path, to path) and the ids the
  moved entries end at; marked neighbours move as a group. The app replays each step as delete plus insert.
- Indenting opens the new parent so the moved entries stay in sight.

## Acceptance tests

- [x] `entries_move_one_place_as_a_group`, `entries_go_in_and_out_with_the_entries_under_them`.
- [ ] Moving a subtree of 500 entries deep in a 10,000-entry outline keeps every child (needs a corpus file).
- [ ] Drag and drop (before, after, into): NSH v0.5.0's `Tree` has no drag and drop, and NSH has no element drag;
  waits for NSH.
