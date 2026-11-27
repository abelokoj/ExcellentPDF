# Ph3: 34/49 | A.21: Comment threads with replies and resolve

**Tag:** A.21; **Priority:** P1; **Phase:** 3; **Needs:** A.4; **Code:** `ecclesia-features::annotate::threads`,
`ecclesia-app::annotate::thread`, `ecclesia-core::annot` (`reply_to`, `state`, `Status`, `page_replies`), the shim's
`ex_annot_reply` and `ex_delete_thread`
**Plan:** §27 entry `#### Ph3: 34/49 | A.21`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reviewers answer each other's comments and settle them. Without threads, an answer becomes a new note stacked on the
page, and whether a comment was accepted or done lives outside the file.

## User story

As a reviewer, I want to reply to a comment and mark it accepted, rejected, cancelled or completed, and have Acrobat
users see the same thread, and see theirs here.

## UI sketch

- Open an annotation (click it on the page or in the list): under its own controls, the inspector shows **Status**
  (Accepted, Rejected, Cancelled, Completed, None), **Replies** (each with who wrote it and when, oldest first, or
  "No replies yet."), a **Your reply** field and **Reply**.
- The annotation list's second line reads, for example, "Highlight; replies: 2; Accepted".

## Mode

Annotate and Review (anywhere the inspector shows an annotation).

## Speed and size impact

Nothing at startup. Opening an annotation reads its page's replies once per edit; the list reads them with the rest.

## Design

- **Model** (`annotate::threads`, pure): `reply`, `set_status` (none when the status is already set), `thread`
  (replies however deep, oldest first, statuses left out, a loop of replies ends), `status_of` (the latest Review
  status; None takes it back; Marked ticks aren't a status), `count`.
- **Core:** `Annotation::reply_to` and `state: Option<Status>`, in the file and the operation log;
  `page_annotations` leaves replies out and `page_replies` reads them; `load_all` reads both for the list.
- **Engine:** the shim writes `/IRT` without `/RT` and, for a status, `/State`, `/StateModel` (`Review`, or `Marked`)
  and the Hidden, Print, NoZoom and NoRotate flags; a reply gets no popup. Display lists and `ex_render_page` skip
  replies. `ex_annot_delete` deletes the thread under an annotation first, in the same operation (DD19).
- **App:** `thread_view` in the inspector; the list row's detail line.

## Acceptance tests

- [x] Written as Acrobat writes it and read back after a save, not drawn, deleted with its parent, back on undo:
  `a_thread_survives_save_and_goes_with_its_parent`.
- [x] Model: `a_reply_is_a_note_on_its_parent_that_names_it`, `a_thread_reads_its_replies_however_deep_oldest_first`,
  `a_loop_of_replies_ends`, `the_latest_status_counts_and_clearing_takes_it_back`.
- [x] List: `replies_are_no_rows_but_counted_and_found_on_their_parent_s`.
- [ ] Threads and states show in Acrobat's comment pane, and Acrobat's replies show here: not checked (no Acrobat
  here).
- [ ] Tried by hand: not yet.

## Open

- Replies can't be edited or deleted one by one yet; deleting the parent takes the thread.
- A.16's transfer leaves replies out (DD19).
- Reply and Set status have no shortcut or palette command (DD19).
