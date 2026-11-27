# Ph1: 57/60 | U.2: Canvas accessibility on Windows

**Tag:** U.2; **Priority:** P0; **Phase:** 1; **Needs:** K.8, K.13; **Code:** `ecclesia-canvas-win::uia`,
`ecclesia-canvas-win::canvas::access`, `ecclesia-text::walk`
**Plan:** §27.18 entry `Ph1: 57/60 | U.2`; §15.

## Problem

The page canvas is a window of its own that NSH doesn't describe, so a screen reader sees one blank pane where the
document is. A blind reader can't read the text, find the links or hear the notes.

## User story

As a reader who uses Narrator or NVDA, I want to read a page line by line, word by word, select text, follow its
links and hear the notes on it, as I do in any document window.

## UI sketch

Nothing new on the screen. To a screen reader the canvas is a **Document** named "Pages of *file*"; its children are
the pages ("Page 3 of 10", type "page"), each holding its links (Hyperlink: their words, or "Link to page 7", or the
web address) and its notes (type "note": the annotation's text). The document has the Text pattern, so the reader's
own keys (NVDA's line, word and character reading; Narrator's scan mode) read the text in reading order.

## Mode

Read mode (the only canvas mode so far); the provider is the canvas's, so later modes keep it.

## Speed and size impact

- Nothing runs until a screen reader asks: `WM_GETOBJECT` makes the root, and events are posted only while
  `UiaClientsAreListening`.
- Notes are read with links, for the pages near the screen: one more small job per page (**not measured**). Its
  `shortcut:` comment names the upgrade (skip it when no screen reader listens).
- Installer: a few kilobytes of code.

## Design

- **Model** (`ecclesia-text::walk`, pure Rust): `DocumentText` walks a document's text by unit (character, word,
  line, paragraph, page, document) over positions `TextPos { page, index }`: `unit_start`, `unit_end`, `expand`,
  `move_by` (with UIA's counting: a unit's start can't be the document's end, an endpoint can), `text` and `spans`.
  A page whose text isn't read counts as an empty page. `PageText::within` finds a link's words.
- **Canvas side** (`canvas::access`): what the providers ask: pages and their client areas, links, notes, text
  areas per line, the caret at a point, the selection, select, scroll to text, follow a link.
- **Providers** (`uia`): `Root` (fragment root, Document, Text pattern; hosted by the window's own provider),
  `Item` (page, link or note; Invoke on links), `Range` (`ITextRangeProvider`). They hold the window handle only and
  find the canvas through a per-thread table, so a gone canvas answers `UIA_E_ELEMENTNOTAVAILABLE`, and a busy one
  too, never a crash. COM threading brings every call to the window's thread.
- **Events**: text changed (pages, links or notes read; a document opened or closed) and selection changed (a mouse
  selection ended, a page selected, a screen reader's Select), posted to the window as `WM_UIA_EVENT` and raised
  outside the canvas's borrow.
- **Strings**: six keys under `canvas.*` in every locale (U.7), read with NSH's `t!`.
- Not built, each recorded in the module's `shortcut:` note: text of pages far from the screen (they read as empty
  until the reader scrolls there; "say all" across a long document waits for K.13's document-wide text); `FindText`
  and text attributes (font, size; answered "none" and "not supported"); tagged PDFs' structure and alt text (K.13
  step 2 reads the structure tree); form fields (they come with the forms feature).

## Acceptance tests

- [x] `units_run_from_one_start_to_the_next_across_pages` (the model: units across pages, unread pages, text)
- [x] `moving_counts_the_starts_it_passes_and_stops_at_the_ends`
- [x] `positions_from_an_older_layout_are_clamped`
- [x] `automation_ids_name_their_page_and_part` (how `RangeFromChild` finds a link's words)
- [ ] NVDA reads a page in order, follows links and reads annotation notes: built; **not tried by hand** with NVDA or
  Narrator, and the headless driver has no screen reader. Not yet checked either: that NSH's window tree shows the
  canvas window to UI Automation as a child (it is a popup NSH adopts).
