# Ph3: 28/49 | A.15: Annotation recovery

**Tag:** A.15; **Priority:** P0 (parity); **Phase:** 3; **Needs:** K.15; **Code:** `excellent-core::document`,
`excellent-core::oplog`, `excellent-app::editing`, `excellent-app::recovery_bar`
**Plan:** §27 entry `#### Ph3: 28/49 | A.15`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

K.15 kept every edit in a log and offered it back after a crash, but in a dialog that asked before showing anything:
the reader had to decide blind, and couldn't see what would come back.

## User story

As a reader whose laptop died mid-review, I want my notes back when I open the file, with a clear note of what came
back, and the choice to keep them or not.

## UI sketch

- The file opens with the edits already on its pages. A bar above the pages, in the accent colour, reads: "The app
  closed before these changes were saved, so they're back: Highlight (6), Sticky note (4). Keep them, or discard
  them and keep the file as it is on disk."
- Discard (secondary) and Keep (primary) at its end. Keep saves them; Discard puts the file back as it is on disk.
  Any edit to the file keeps them as well, and the bar goes.
- Hidden in full screen; shown again on leaving it, until decided.

## Mode

All modes.

## Speed and size impact

None at startup: the log is read when its file opens (as K.15 did). Replaying it runs on the UI thread, as before.

## Design

- **Core:** `oplog::Read::restored` names the edits the saved file lacks; `Document::recover` returns them;
  `Document::revert` reopens the file and restarts the log.
- **App:** `Editing` recovers when a file is shown, keeps the names per file until the reader decides, and saves
  nothing before; `keep_recovered`, `discard_recovered`. `recovery_bar.rs` draws the bar and counts each kind.

## Acceptance tests

- [x] `ten_annotations_drawn_before_a_crash_all_come_back` (the Done when; also Discard).
- [x] `unsaved_edits_come_back_after_a_crash` (lists only what the saved file lacks).
- [x] `the_bar_counts_each_kind_of_change_in_the_order_first_made`.
- [ ] Tried by hand: not yet.

## Open

- Keep and Discard are the bar's buttons only, without a shortcut or a palette command: the bar appears once, after
  a crash.
- A file changed on disk since its edits were logged still loses them (K.15 step 5, "keep both", is not built).
