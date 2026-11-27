# Ph4: 41/44 | G.6: Whiteboard page

**Tag:** G.6; **Priority:** P0; **Phase:** 4; **Needs:** G.1, N.5; **Code:** `ecclesia-features::present::whiteboard`
**Plan:** §27 entry `#### Ph4: 41/44 | G.6`.

## Problem

During Q&A or live derivations, presenters need an instant blank or grid board without closing the slide deck or altering the slide PDF.

## User story

As a lecturer, I want to pull up a temporary blank or grid whiteboard mid-presentation, sketch freely, return to my slides leaving the original PDF untouched, and optionally save the whiteboard into a notebook afterwards.

## UI sketch

- Present toolbar: Whiteboard toggle (`N.5` infinite canvas), background selector (Blank, Grid, Dot, Lined), Save Whiteboard to Notebook.

## Mode

Present mode.

## Speed and size impact

- Volatile overlay backed by `InfiniteCanvas`; original slide PDF is never written.

## Design

- `WhiteboardDeck` manages `WhiteboardPage` instances with `WhiteboardBackground::{Blank, Grid, Dot, Lined}` and `export_to_notebook(title)` producing a standalone `.expdf` `Notebook`.

## Acceptance tests

- [x] `temporary_whiteboard_leaves_original_file_unchanged_and_can_save_to_notebook`: inserts a grid whiteboard, draws strokes, returns to slides verifying the slide PDF is byte-identical, and exports the whiteboard to a valid `.expdf` notebook.
