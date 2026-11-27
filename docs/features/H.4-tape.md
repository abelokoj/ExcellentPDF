# Ph4: 3/44 | H.4: Tape

**Tag:** H.4; **Priority:** P1; **Phase:** 4; **Needs:** H.2; **Code:** `ecclesia-features::annotate::tape` (the
model; the plan's `ecclesia-ink::tape` holds no geometry of its own, so the tape lives with the other annotation
models), `ecclesia-core::annot` (`Annotation::tape`, kept in the private key), `ecclesia-app` (`ink.tape`)
**Plan:** §27 entry `#### Ph4: 3/44 | H.4`.

## Problem

Testing yourself from your notes means covering the answers; a strip of paper over a printed page does that, a PDF
reader didn't.

## User story

As a student revising, I want to tape over definitions, answers and vocabulary, then tap each strip to check myself,
and tap it again to cover it for next time.

## UI sketch

Tape on the Write toolbar (Alt+T on the pages, the palette): the next drag on a page lays a soft blue strip over what
it covers. A click on tape shows what is under it (the strip turns faint) and another covers it again; Ctrl-click,
the marquee and the lasso still select tape to move, resize or delete it.

## Mode

Write (the tool); a tap works in every mode.

## Speed and size impact

None: one annotation per strip.

## Design

- **Saved:** a standard Square annotation, filled and opaque in the tape's colour, with `"tape": true` in the app's
  private key: other readers show a plain bar.
- **A tap:** one undoable change of the strip's opacity, between covering (1) and revealed (0.18, enough to see where
  it is). It is saved with the file, so a page comes back as it was left; a locked strip stays as it is.
- **Laid:** the rectangle drag of A.8; the inspector doesn't open for tape.

## Acceptance tests

- [x] Tape toggles with one tap and survives save: `tape_toggles_with_one_tap_and_survives_save` (a real PDF: laid
  covering, one tap reveals it, saved and reopened still tape and revealed, another tap covers it).
- [x] `tape_is_an_opaque_filled_square_marked_as_tape`, `a_tap_reveals_and_another_hides_again`,
  `only_tape_and_unlocked_tape_answer_a_tap`.

## Open

- "Reveal all" and "cover all" on a page are not built; exam mode (S.10) will use tape.
