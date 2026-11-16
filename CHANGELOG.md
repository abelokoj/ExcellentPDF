# Changelog

All notable work on ExcellentPDF is recorded here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), with one dated entry per piece of work. Times are local
(UTC−05:00) and mark when the work was finished.

## [Unreleased]

### 2026-11-16 11:42; Ph3: 38/49 | A.28: W3C Web Annotation export

- Export and Import annotations (Ctrl+Alt+E, Ctrl+Alt+I) now also take W3C Web Annotation files (`.jsonld`), the
  format web annotation tools share. Highlights carry text-quote and text-position selectors and a page fragment;
  comments, tags and review states travel as bodies, replies as replies. Our own files come back whole and merge by
  id; another tool's highlights are placed by finding the text they quote.

### 2026-11-16 11:08; Ph3: 37/49 | A.25: Flatten on export

- "Export flattened copy" (Ctrl+Shift+E, the Annotate toolbar's Flatten, or the palette) saves a copy of the document
  with its annotations drawn into the pages: every reader shows them, and nobody can move or delete them. The open
  document keeps its annotations, edits and undo history; the copy can't replace it.

### 2026-11-16 10:39; Ph3: 36/49 | A.24: XFDF and FDF import and export

- Annotations can be exported as XFDF or FDF (Ctrl+Alt+E, the Annotate toolbar's Export, or the palette), the files
  Acrobat and other PDF tools import.
- Importing an XFDF or FDF file (Ctrl+Alt+I) merges it by annotation id: annotations already here are updated, new
  ones added, locked ones left alone. The whole import is one step of undo, and a summary says what changed.
- Replies, statuses, groups, colours, borders, fills, clouds, line ends, text boxes and callouts all travel; tags,
  categories and layers ride along in attributes other tools ignore.

### 2026-11-16 09:59; Ph3: 35/49 | A.22: Annotation layers by person or purpose

- Annotations can be put on layers, the PDF's optional content groups that Acrobat lists in its Layers panel. The
  inspector's Layer row takes a layer name (a new one makes the layer) or a chip of an existing layer.
- The top of the annotation list (Ctrl+Alt+L, or "Layers" in the palette) lists the layers: show or hide each, see
  how many annotations it holds, and lock it so its visibility stays. Each change is one step of undo and is saved in
  the file, so other readers open it with the same layers hidden.
- "One per author" puts every annotation on no layer on its author's layer; from then on a reviewer's new
  annotations go on their layer by themselves.
- A hidden layer's annotations are not drawn, pressed or shown in the margin, but stay in the annotation list.

### 2026-11-16 09:26; Ph3: 34/49 | A.21: Comment threads with replies and resolve

- An annotation's inspector now shows its comment thread: a status (Accepted, Rejected, Cancelled, Completed, None),
  its replies oldest first with who wrote them and when, and a field to reply. Each reply and each status is one
  step of undo, and a locked annotation can still be answered.
- Replies and statuses are written as Acrobat writes them: a note whose `/IRT` names its parent; a status also has
  `/State` and `/StateModel` and is hidden. Replies made in Acrobat show here.
- Replies are not drawn or clickable on the page. The annotation list shows each thread's reply count and status, and
  its search finds an annotation by its replies' words.
- Deleting an annotation deletes its thread with it, and undo brings both back.

### 2026-11-16 09:01; NSH 0.5.1

- NSH updated from 0.5.0 to 0.5.1 (D2): the workspace pins, the generated `app/.nsh` crates and both lockfiles; the
  local tool in `C:\dev\nsh` and the source in `C:\dev\nsh-src` are at 0.5.1 too. No code changes were needed.

### 2026-11-16 08:23; Ph3: 33/49 | A.20: Stamps library with dynamic fields

- A Stamp command (toolbar, Shift+M on a page, the palette) puts a stamp where the page is clicked next. While it
  waits, the inspector offers the PDF standard's fourteen stamps, five dynamic stamps that carry the reader's name,
  the date, the time (UTC) and the page, and the reader's own stamps.
- Import adds a PDF's first page or a PNG or JPEG image to the library, kept in the app's data folder under `stamps`;
  Remove takes the chosen one out again.
- A stamp and its art are one step of undo. Dynamic and library stamps keep their own appearance in the file, so
  other readers show them as drawn.

### 2026-11-16 05:56; Ph3: 32/49 | A.19: Filters by color, tag, author and date

- The Annotations tab filters by tag, by author (when a file has more than one) and by when an annotation last
  changed (today, the past week, the past month), beside the kind, colour and category filters. Filters combine:
  every one chosen must match; within one, any choice may.
- "Show only matching on the pages" leaves the annotations the list doesn't show out of the pages too; the file is
  not changed.
- Each file's filter is kept in the library and is there again when the file is opened. "Clear filters" shows
  everything again.

### 2026-11-16 05:31; Ph3: 31/49 | A.18: Tags on annotations

- An annotation's inspector has a Tags field: type tags separated by commas, and they are saved inside the PDF with
  the annotation (one step of undo).
- The Annotations tab's search finds tags, and `tag:method` finds the annotations tagged "method" only.
- Each time a PDF is saved, its annotations' tags are recorded in the library, ready for searching every document
  by tag when library search comes (Phase 5).

### 2026-11-16 05:20; Ph3: 30/49 | A.17: Semantic categories

- An annotation can now be a Claim, Evidence, Question, Definition, Method, Result or To do, each with its own colour.
  Pick one in the annotation's inspector; press it again to take it away. Each change is one step of undo.
- The category is saved inside the PDF with the annotation, so it is there after a save and a reopen.
- The Annotations tab shows each row's category and, when the document has any, offers category chips to show only
  those.

### 2026-11-16 05:10; Ph3: 29/49 | A.16: Transfer annotations to another version

- Transfer annotations from another version (Annotate toolbar, `annotate.transfer`, Shift+V on the page) takes the
  annotations of a file you pick, such as a paper's preprint, into the open PDF.
- Each highlight, underline or strikeout is found again by the text it marks, on whatever page that text is now, even
  with small changes to it; notes, shapes and drawings keep their page and place.
- Before anything is added, a review says how many were found where they were, found elsewhere, kept in place and
  not found, and names the first that weren't. Adding them is one step of undo.

### 2026-11-16 04:56; Ph3: 28/49 | A.15: Annotation recovery

- When the app closed before your changes to a PDF were saved, opening the file brings them back at once, and a
  "Recovered changes" bar above the pages lists them by kind ("Sticky note (10)").
- Keep saves them like any edit; Discard drops them and leaves the file as it is on disk. Editing the file keeps them
  too. Close it undecided and they come back next time.
- This replaces the dialog that asked before restoring anything.

### 2026-11-16 04:47; Ph3: 27/49 | A.14: Annotation list with filter and search

- An Annotations tab beside the inspector lists every annotation of the shown document under a heading per page:
  a bar in its colour, its note or the text it marks, and its kind. It is read in the background and again after
  every edit, and scrolls smoothly however long it is.
- A search field finds words in the notes and the marked text as you type (every word, any case); chips show only
  some kinds or one colour.
- Pressing a row shows its page, selects the annotation and opens its note.
- Open it from the Annotate toolbar (List), the palette (`annotate.list`) or Shift+O on the page.

### 2026-11-16 04:39; Ph3: 26/49 | A.13: Select, move, resize, copy, paste and delete

- A click on an annotation selects it (outlined in blue, its note open); Ctrl-click adds or removes one, and a drag on
  an empty part of the page draws a marquee that selects what lies wholly inside it.
- Drag a selected annotation to move it (with the rest of the selection), a corner to resize a box, an ink drawing
  or a callout, or a grip on a line's end or a polygon's corner to move that point. Highlights and other text markup
  stay with their text; locked annotations are outlined but don't move.
- The arrow keys nudge the selection by a point; Delete removes it (Annotate toolbar, `annotate.delete`); Ctrl+C,
  Ctrl+X and Ctrl+V copy, cut and paste it, in the same document or another, keeping kind, look and note. Each is
  one step of undo. The notes go on the system clipboard as text for other apps.
- Escape clears the selection.

### 2026-11-16 04:08; Ph3: 25/49 | A.12: Annotation inspector

- The inspector now shows every annotation's author (editable), colour (the palette's swatches), opacity (25 to
  100 %), when it was made and last changed, and a Locked switch, under its note and its kind's own controls.
- Each change is one step of undo, and the inspector is read again after every edit, so an undo shows at once.
- A locked annotation (PDF's Locked flag, which other readers honour) keeps its look and can't be deleted from the
  inspector; its note can still be written.
- Category, tags and layer join the inspector with A.17, A.18 and A.22; editing several at once waits for A.13's
  selection.

### 2026-11-16 03:51; Ph3: 24/49 | A.11: Margin notes and anchored note cards

- Margin notes: a column beside the pages shows each note on the pages in view as a card (its colour, kind, author
  and first lines), level with what it is about. Cards that would overlap are pushed apart round the middle of their
  notes, and follow the pages as they scroll, zoom and turn.
- A click on a card opens its note to write in place (and in the inspector); Done saves it as one step of undo.
- `annotate.margin_notes` (`M` on the pages, palette, Annotate toolbar) shows or hides the column; also in Settings,
  Annotation. Off by default.
- The column is beside the pages rather than over them (dev decision DD11).

### 2026-11-16 03:25; Ph3: 23/49 | A.10: Polygons, polylines and clouds

- Polygons, polylines and clouds: three commands (palette, Annotate toolbar, `P`, `Shift+P` and `Shift+K` on the
  pages). Each click puts down a corner, shown with the side to the mouse and, for a polygon or cloud, the side that
  will close it; a double-click finishes, as does a click on a polygon's first corner. Shift keeps a side at 15° steps.
- Regular polygons and stars (`Shift+R`, `Shift+X`): dragged out like a rectangle, tip up. Their side and point
  counts (3 to 32, 5 by default) are in Settings, Annotation.
- In the inspector, a closed shape (rectangle, ellipse, polygon, cloud) gets a Border row, Plain or Cloudy, beside its
  fill; a polyline gets the line's ends.
- Saved as standard `/Polygon` and `/PolyLine` annotations with `/Vertices`; a cloud is a polygon with `/BE` `/S/C`
  `/I 1`. A border's effect is rewritten only when it changes.

### 2026-11-16 01:27; Ph3: 22/49 | A.9: Rectangles and ellipses

- Rectangles and ellipses: two commands (palette, Annotate toolbar, `R` and `Shift+E` on the pages) turn on the
  drawing tool; a drag on a page draws the box from corner to corner, shown as it goes, and Shift makes it a square
  or a circle. Each is one undoable edit in the shapes' colour, and its inspector opens.
- In the inspector, for every shape (lines and arrows too): line width (1, 2 or 4 pt) and line style (solid, dashed,
  dotted); for a rectangle or ellipse, a fill (none, white, yellow, blue, red). Each change is one step of undo.
- Saved as standard `/Square` and `/Circle` annotations: the fill as `/IC`, the dashes as `/BS` with `/S/D`. Another
  app's beveled or inset border is left alone unless its dashes are changed here.
- A click on a shape's border, or inside a filled box, opens it again.

### 2026-11-16 01:01; Ph3: 21/49 | A.8: Lines and arrows

- Lines and arrows: two commands (palette, Annotate toolbar, `Shift+L` and `Shift+A` on the pages) turn on a drawing
  tool; a drag on a page draws the line, shown as it goes with a crosshair pointer, and Shift snaps it to steps of
  15°. The line is one undoable edit, 2 points wide in the shapes' colour (a new tool in the colour dialog, red to
  start), and its inspector opens.
- In the inspector: the start and the end each take an ending (none, open or closed arrow, dot, square, diamond,
  slash), written as `/LE` so Acrobat draws the same heads. A click on a line opens it again; Delete line removes it.
- The page canvas has a `drawing` prop and sends `pageDraw`; the host tells it whether Shift is down (Windows reads
  the key; the others draw unsnapped for now).
- The model is `excellent-features::annotate::shapes`.

### 2026-11-16 00:39; Ph3: 20/49 | A.7: Callouts with leader lines

- Callouts: a command (palette, Annotate toolbar, `O` on the pages) waits for a click and puts a text box beside the
  point clicked, above and to the right or wherever the page has room, with a leader line from the box to that
  point (`/IT /FreeTextCallout`, `/CL`). Its text, font, size, alignment, colour and border are edited as a text
  box's; the inspector adds the arrow at the tip (open, closed, dot, square or none).
- Moving or growing the box keeps the tip where it is: the model keeps the tip (`Shape::Callout`), and the shim
  writes the line from it, so MuPDF draws it again to the box's new edge.
- The model reads and writes line endings (`LineEnd`, `/LE`) for lines, polylines and polygons too, ready for A.8.
- MuPDF's rect setter gives every free text a leader line to the page's corner; the shim now removes it from text
  boxes that never had one.

### 2026-11-16 00:25; Ph3: 19/49 | A.6: Text boxes and typewriter

- Text boxes and typewriter text: two commands (palette, Annotate toolbar, `T` and `Shift+T` on the pages) wait for
  a click and put a free-text annotation there, then open its text in the inspector. A text box is 200 points wide
  with a border and grows down as its text wraps; typewriter text (`/IT /FreeTextTypeWriter`) has no border and its
  box fits the text, for filling forms that have no fields.
- In the inspector: the text (typed in an NSH field, so input methods work), font (Helvetica, Times, Courier: the
  standard fonts every reader has), size, alignment, colour and, for a box, its border. Each change is one undoable
  edit and fits the box again. A box placed and left empty is taken away; Delete text box removes one.
- A click on a text box opens its text, as a click on a markup opens its note.
- The annotation model keeps a text box's style (`TextStyle`: `/DA` and `/Q`) and typewriter text as its own kind;
  the shim reads and writes the intent, alignment, font, size and colour, and leaves another app's rich text alone
  when the style is unchanged. A test saves Chinese typewriter text, checks the bytes Acrobat reads, and reopens it.
- The model is `excellent-features::annotate::freetext`.

### 2026-11-16 00:04; Ph3: 18/49 | A.5: Sticky notes

- Sticky notes: the command (palette, Annotate toolbar, `Shift+S` on the pages) waits for a click on a page and puts
  a 20-point note icon there, kept on the page, then opens its note in the inspector. Run it again to call it off.
- In the inspector a sticky note has its icon, one of the PDF standard's seven (Comment, Note, Help, Insert, Key,
  Paragraph, New paragraph), and whether other readers show it open. Each change is one undoable edit.
- A click on a sticky note opens its note, as a click on a markup does (A.4).
- The annotation model keeps the icon (`/Name`, as the file has it) and the open state (`/Open`, the note's own or
  its popup's); the shim reads and writes both. A test saves, reopens and checks the bytes Acrobat reads.
- The model is `excellent-features::annotate::sticky`.

### 2026-11-15 23:48; Ph3: 17/49 | A.4: Notes on highlights

- A markup's note is its PDF `/Contents`; the shim gives a noted markup a `/Popup` beside it, so Acrobat and other
  readers list it in their comment panes. Saved, reopened and read back by a test.
- A click on a highlight, underline, squiggly or strikeout opens its note in the inspector, beside the pages: the
  markup's kind, page and colour, the text it marks, and the note. Done saves it as one undoable "Note" edit; Delete
  note clears it. Opening another note saves the first.
- The markup bar has a Note tool (a speech bubble) after the proposed edits: it highlights the selection and opens
  the new highlight's note. Also in the palette, the Annotate toolbar and `Shift+N` on the pages.
- The model is `excellent-features::annotate::notes`.

### 2026-11-15 23:19; Ph3: 16/49 | A.3: Annotation colors

- Each markup tool (highlight, underline, squiggly, strikeout, and the replace and insert proposals) has a colour and
  an opacity of its own. Highlighters start from the theme's light annotation colours, the line tools from strong
  inks.
- The markup bar draws each tool in its colour and shows the current tool's (the last one pressed) five swatches and
  a custom cell. A swatch recolours that tool and applies it; the custom cell opens the colour dialog.
- The colour dialog (palette, Annotate toolbar, `Shift+C`, the settings window) has a tool selector, NSH's colour
  picker with a saturation and brightness area, a hue slider, an opacity slider and a hex field, the palette's
  swatches and the custom colours used lately. Done keeps the changes; Cancel or Escape drops them.
- Settings, Annotation: the palette, Standard or Colour-blind safe (Okabe and Ito's colours). Everything is kept in
  `settings.toml` under `[annotation]`.
- The model is `excellent-features::annotate::colors`.

### 2026-11-15 23:04; Ph3: 15/49 | A.2: Replace and insert text

- The markup bar under selected text has two more buttons after a rule: Replace (strikeout and caret) and Insert
  (caret). Replace asks for the new text in a small dialog, starting from the selected text, and proposes it as a
  strikeout over the selection grouped with a caret holding the new text, the pair Acrobat shows as one "replace
  text" comment. Insert asks for text and leaves a caret holding it after the selection, or where the last click in
  the text put the cursor.
- Both are commands (the palette; Annotate mode's toolbar) with page keys `E` (replace) and `Insert`; each is one
  undoable edit, saved into the PDF.
- Annotations now keep the annotation they are grouped with (`/IRT` with `/RT /Group`) through save, reopen and the
  operation log.
- The model is `excellent-features::annotate::replace`; the shim writes and reads the group.

### 2026-11-15 22:34; Ph3: 14/49 | A.1: Text markup: highlight, underline, squiggly, strikeout

- Selecting text in a PDF floats a small bar under the selection (over it when there's no room below): highlight,
  underline, squiggly underline and strikeout, then five colours (yellow, green, blue, pink, orange). A markup
  button marks the selection in the chosen colour as one undoable edit, saved into the PDF as a standard markup
  annotation, and clears the selection.
- The same four are commands (the palette; Annotate mode's toolbar) with page keys `a` (as SumatraPDF), `u`,
  `Shift+U` and `x`.
- Marking text that touches a markup of the same kind and colour on the same line extends it instead of stacking a
  second one; text that bridges two such markups folds them into one.
- Each markup keeps the text under it, with a little context either side (K.28), to find it again in another
  version of the document.
- The page canvas now draws again the pages an edit, undo or redo changed (it kept the old drawing before).
- The model is `excellent-features::annotate::markup`; the bar is the canvas's `markbar`; the app's side is
  `annotate`.

### 2026-11-16 02:20; Ph3: 13/49 | W.11: Export and import

- "Export workspace…" (the palette; the workspace list's actions) saves the shown workspace as a small JSON file:
  its name, icon and colour, its panes' layout, and each pane's tabs with their views, pins and groups. A tab's file
  under the folder the workspace file is saved in is written relative to it, with `/`, so the two can move together
  or go to another computer; any other path stays as it is. Closed tabs stay behind.
- "Import workspace…" makes a new workspace from such a file and shows it. Tabs whose files aren't where the file
  says come in greyed out with "Locate…" (R.9), and a notice says how many.
- A file that isn't a workspace, is larger than 8 MB, or comes from a newer version is refused with a notice; a
  file's places that point past its lists are dropped rather than trusted.
- `excellent-store::workspace_file` is the format, a pure module; the app's side is `workspaces::file`.

### 2026-11-16 01:30; Ph3: 12/49 | W.9: Search tabs across workspaces

- The tab list (Ctrl+Shift+A, the palette, the strip's list button) has a section for each other workspace with
  tabs, in the workspaces' order: "In Tax (12)", or "In Tax, another window (12)" when another window shows it.
  Typing finds a tab by its file's name, its folder or its workspace's name.
- Return on one switches to its workspace and shows the tab, making its pane the active one; when another window
  shows that workspace, that window comes to the front with the tab shown. A tab closed since it was saved opens
  again.
- The store reads every other workspace's open tabs in one query (`workspaces::tabs_elsewhere`).

### 2026-11-16 00:40; Ph3: 11/49 | W.8: One workspace per window

- "Open workspace in new window" (Ctrl+Alt+Shift+N, the palette, the workspace list's actions) lists the other
  workspaces; the one picked opens in a window of its own with everything the main window has: tabs, panes,
  sidebar, palette, dialogs and modes.
- A workspace shows in one window at a time. Switching to one another window shows (the list, Ctrl+Alt+1 to 9),
  opening a file that workspace has, or opening it in a window again brings that window to the front; cycling
  passes over it; a deleted workspace's tabs go to one no other window shows; a tab sent to it lands there at once.
  The list says which workspaces are open in another window.
- Closing a workspace window saves its edits and tabs and frees its workspace; the next start opens the windows that
  were open (`meta` key `workspace.windows`), the main window on its own workspace as before (W.7).
- Full screen (R.19) acts on its own window. The shell is now the app's part (`shell`) and each window's
  (`window_shell`); the workspace list is one signal every window shares.

### 2026-11-15 23:30; Ph3: 10/49 | V.11: Pop out to a window

- "Move tab to its own window or back" (Ctrl+Alt+O) moves the shown tab into a window of its own, at its page and
  in its view. The window has its own tab strip and the main window's shortcuts for its tabs and pages; a file open
  in both windows is read once and shares its edits.
- The same command in that window, its "Dock" button, or closing it, moves its tabs back to the main window's active
  pane; closing its last tab closes it. The session keeps its tabs with the active pane's.
- The page canvas is now one view (`panes::pages_view`) used by the grid and the new windows (`excellent-app::windows`).

### 2026-11-15 22:45; Ph3: 9/49 | V.10: Swap, flip and maximize

- "Swap panes" (Ctrl+Alt+X) trades the active pane's tabs with the next pane's; "Flip panes between side by side and
  stacked" (Ctrl+Alt+F) turns the grid over its diagonal, sizes and tabs going along; "Maximize pane or restore
  panes" (Ctrl+Alt+Enter, a double-click on the pane header, or its new □ button) fills the window with the active
  pane and brings the others back. Each one undoes itself when done again.
- A maximized pane keeps its header; closing a pane, splitting or a pane emptying brings the hidden panes back. The
  session keeps a maximized pane.

### 2026-11-15 21:30; Ph3: 8/49 | V.9: Open a link in the other pane

- Ctrl+click (Command-click on a Mac) or middle-click on a link in the pages shows where it goes in the other pane;
  the pane clicked in stays where it is. With one pane, the window splits. A link out of the document opens as a
  click would.
- In the outline, the middle button, Ctrl+Enter, the entry's menu and the palette ("Open outline entry in other
  pane") do the same; Ctrl+click still marks entries (B.10). A search result opens its page there with the middle
  button.
- The other pane shows the document's tab, or a copy of the clicked pane's tab, and goes there with a history step,
  so Back returns. A tab that opens waits for its file before going (`TabState::show_then`).

### 2026-11-15 20:00; Ph3: 7/49 | V.8: Synchronized scrolling

- Ctrl+Alt+S, or the Sync menu in a split pane's header, makes the other panes follow the one you move: by page, by
  position through the document, or from pages you paired; the zoom can follow too.
- How panes follow and the zoom sync are in Settings, Tabs. The page canvas gained a `follow` command that moves it
  without touching its back and forward history.
- Done when met by test: same-position following stays level across documents of different page sizes. Not tried by
  hand.

### 2026-11-15 18:30; Ph3: 6/49 | B.10: Sort, promote and demote between outline and bookmarks

- "Sort outline entries by page" puts an entry's children, or its level, in page order as one step of undo; entries
  that open no page go last.
- "Add outline entry as bookmark" and "Add bookmark to outline" copy a place between the outline and the reader's
  bookmarks, keeping its title, page, point and zoom; the original stays.
- Done when met by test: a place written into the outline reads back with the same title and destination. Not tried by
  hand.

### 2026-11-15 17:00; Ph3: 5/49 | B.9: Check destinations

- Outline entries that open nowhere say why in their row: no destination, a missing named destination, or a page past
  the last.
- "Check outline destinations" counts them; "Fix broken outline entries" points each at the heading with its title
  (found on the workers) or removes it, as one step of undo.
- Done when met by test: a corpus PDF given an entry past its last page and one with a missing named destination has
  both found. Not tried by hand.

### 2026-11-15 15:30; Ph3: 4/49 | B.7: Bold, italic and color

- Outline entries show the document's bold, italic and colour, the colour kept readable on the theme (WCAG AA).
- The entry menu has Bold, Italic and a Colour submenu (five swatches and none); Ctrl+Shift+B and Ctrl+Shift+I while
  the outline has the keyboard; all in the command palette. One step of undo, saved in the PDF as `/F` and `/C`.
- Shim: `ex_load_outline` reads `/F` and `/C` through the outline iterator; new `ex_outline_style`.
- Fixed (R.14): `ex_display_list_images` read its item list after the device that owned it was dropped, so macOS
  listed no pictures; the list is taken out before the drop.
- Done when met by test: a styled entry keeps bold and colour after save and reopening, with `/F 2` in the file. Not
  opened in Acrobat.

### 2026-11-15 12:30; Ph3: 3/49 | R.25: Citation and equation hover

- Resting the pointer on a citation (`[12]`, `(Smith et al., 2020)`) or an equation reference shows the entry or the
  equation in a preview drawn over the pages; clicking it goes there. Papers without links work too: the entry is
  found in the references by its text.
- `excellent-features::read::refhover`: SumatraPDF Enhanced's reference hover ported to pure Rust (`cite`, `layout`,
  `resolve`) with its tests.
- `excellent-canvas`: a worker per canvas debounces, resolves and renders the preview off the UI thread.
- Settings, Links: "Preview citations and equations" and "Preview delay".
- Done when met by test: the ported tests pass; `[12]` in a paper without links shows reference 12. Not tried by
  hand.

### 2026-11-15 11:45; Ph3: 2/49 | R.17: Mini-map with note ticks

- A strip on the right edge of the pages shows the whole document: a tick per page, a thumb over the part on screen,
  and the reader's highlights, notes, bookmarks and search hits in lanes, each in its colour. Press or drag it to
  scroll.
- `excellent-core`: `annot::page_marks` and `Renderer::load_marks`; operations know their pages, and apply, undo and
  redo bump those pages' revisions, so the strip (and later the pages) redraw only what changed.
- Bookmarks load whenever a document is shown, not only when their panel is open, and reach the canvas as a
  `bookmarks` prop.
- Done when met by test: an edit, undo and redo each change the page's revision and marks, read again the next frame.
  Not tried by hand.

### 2026-11-15 11:25; Ph1: 4/60 | K.14: Annotation model

- `excellent-core::annot`: the annotation model (step 1). Each annotation has its `/NM` id, page, shape (text
  markup, a boxed kind, ink, a line, a polygon or polyline), style, author, dates, contents, tags and K.28 anchor;
  tags and the anchor are kept in a private `/ExcellentPDF` key that other readers ignore.
- Creating, changing and deleting an annotation are operations in the log, undone and redone like outline edits.
- Done when met: a model test of 10,000 random edit sequences, each saved, reopened and compared with the model, then
  undone back to the start. Still open: undo after reopening.

### 2026-11-15 10:50; Ph3: 1/49 | K.28: Annotation anchors

- `excellent-core::anchor`: W3C text-quote anchors (prefix, exact text, suffix, start) and `locate`, which places a
  quote in a changed document exactly, approximately (at most a quarter of its characters wrong, widened to whole
  words) or not at all ("orphaned", for the reader to place). Line breaks, hyphenation, ligatures and curly quotes are
  normalized away first, so a re-typeset version usually matches exactly.
- `new_id()` gives version 4 UUIDs for `/NM`.
- Done when met on the test set: at least 95 % of quotes land on the right words in a re-typeset, corrected version.
  Storing anchors in annotations waits for the annotation model (K.14 step 1).

### 2026-11-15 10:40; Ph1: 55/60 | O.22: Printing on macOS and Linux

- macOS: Print shows `NSPrintPanel` over the app's shared print info; Linux: GTK's print dialog (the portal under
  Flatpak). Both run on the main thread while the job's thread waits.
- The job's bands go to a PDF spool (one image per band, Flate-packed, written as they come), which PrintCore or
  GTK sends to the printer chosen, with its settings; the spool is deleted afterwards. Same app code on all three
  desktops.
- Fixes for the macOS canvas build (CI): `NSTrackingArea` is allocated off the main-thread path; the frame's
  `CGContext` is converted to the host's type. The Linux canvas lost an unused getter.
- Budgets: each launch of the open-a-PDF run gets its own profile, so none restores the last run's session (on the
  CI runner a later launch sometimes drew nothing; it never does locally).

### 2026-11-15 10:23; Ph2: 3/3 | U.3: Canvas accessibility on macOS and Linux

- `excellent_text::FlatText` lays the document's text out as one string (a line break after each line, unread pages
  adding nothing) and maps offsets to page positions and back; the shared canvas answers units, the selection, the
  pages shown and screen areas in those offsets.
- Linux: the GTK canvas widget implements `GtkAccessibleText`, so Orca reads the pages by character, word and line
  and follows the selection; changes are announced on idle.
- macOS: the canvas view is an accessibility text area (value, selection, visible range, lines, strings and frames
  for ranges), posting value and selection changes to VoiceOver.
- AccessKit was weighed and not used: each toolkit already owns its platform tree.

### 2026-11-15 10:41; Ph2: 1/3 | K.9: Page canvas on macOS (AppKit view)

- `excellent-canvas-mac` is the shared canvas's AppKit host: a flipped, layer-backed `NSView` drawing each frame with
  Core Graphics in `drawRect:` (DD10), tiles as `CGImage`s, darkest-wins highlights through `kCGBlendModeDarken`,
  workers waking it on the main dispatch queue.
- Input: trackpad and wheel scrolling (Command zooms, Shift scrolls sideways), pinch zoom, the mouse (links,
  selection, middle-click scrolling, back and forward buttons) and the pen's tablet events (pressure; the eraser end
  from proximity). A page point is a screen point at 100 %, as in Preview.
- The app registers it on macOS (`#[native_components(macos)]`).

### 2026-11-15 10:06; Ph2: 2/3 | K.10: Page canvas on Linux (GTK widget)

- `excellent-canvas-gtk` is the shared canvas's GTK 4 host: frames are GSK render nodes drawn in the widget's
  `snapshot`, tiles are `GdkMemoryTexture`s, darkest-wins highlights blend with `GskBlendMode::Darken`, and workers
  wake the widget through the main context.
- The widget follows its size and the surface's (fractional) scale, and takes the wheel and touchpad (Ctrl zooms,
  Shift scrolls sideways), the mouse (links, selection with the system's double-click limits, middle-click
  scrolling, back and forward buttons) and the pen through `GtkGestureStylus` (pressure, eraser end).
- The app registers it on Linux (`#[native_components(linux)]`), so the Linux build shows pages.
- CI: the 20 ms cancel test allows 100 ms when `CI` is set; shared runners in debug builds missed it at 32 ms.

### 2026-11-15 06:48; Ph2: 2/3 | K.10: Page canvas on Linux (shared canvas core)

#### Changed

- **One page canvas for every platform** (DD9): the canvas's logic moved from `excellent-canvas-win` to
  `excellent_canvas::page`, as `PageCanvas<H: Host>`, with the component's props and commands. Windows is now one
  host of it; GTK and macOS follow. The reader's view model moved from `excellent-core` to `excellent-canvas`.
- The size budget's baseline is 13.98 MB, up from 11.79 MB at Phase 0 (still under the 15 MB limit); the budget
  script prints the app's log when a run draws no first page.

#### Fixed

- Tests: the file-link test checks drive letters only on Windows and rooted paths on macOS and Linux, where it failed.

#### Not done

- Nothing tried by hand on Windows after the move; checked by clippy and the tests only.

### 2026-11-15 05:37; Ph1: 8/60 | K.18: Modes and toolbars

#### Added

- **Modes**: a bar under the tabs switches between Read, Annotate and Present (Write and Study join once they have
  tools), and shows only the chosen mode's tools. The mode's colour runs along the bar and the shown tab.
  Ctrl+Shift+M and Ctrl+Alt+Shift+M step through the modes; each mode is also in the palette.
- **Customize toolbar**: add, remove, drag or arrow-move each mode's tools and separators; **Reset** brings back the
  defaults. **Export…** and **Import…** share every mode's toolbar as a JSON preset; a preset from a newer version, or
  a file that isn't one, is refused with a reason.
- **Pen switches to Write** (Settings, Modes and toolbars; on by default): a pen's touch on a page switches to Write
  once Write has tools.
- The getting-started guide's second step now names the modes (DD7).

#### Not done

- Write and Study have no tools yet; tools are text, not icons (DD8). Ink still draws in any mode until H.1. Not tried
  by hand.

### 2026-11-15 05:18; Ph1: 58/60 | U.4: High contrast, scaling and reduced motion (fixes)

#### Fixed

- **Narrow windows**: below 900 points of width (a 1280-pixel screen at 150 % and up) the sidebar and inspector open
  as drawers over the page instead of squeezing it; with both docks open the page used to get under 100 points.
- **Screen readers** (U.2): a text position on a page the document no longer has now counts as the document's end, not
  its start, so a range reaching past the last page still reads that page's text.
- Tests: key caps such as ⌘K no longer count as untranslated text, and the keyboard test closes the go-to box with
  Escape, since the headless window lays out no pages.

### 2026-11-15 03:59; Ph1: 60/60 | U.9: First-run guide and tips

#### Added

- **Getting started**: the first time the app starts without files, three short steps show how to open a document,
  where the reading options are, and the command palette, with the reader's own keys. **Skip** or Escape closes it
  for good; **Open a document** goes straight to the file dialog. A start with files never shows it, and files from a
  later launch close it.
- **Tips**, once each, in a strip below the pages: the palette the first time a document shows, the tab list the first
  time two tabs are open. **Turn off tips** there, or **Settings, Getting started, Show tips**.
- **Show the getting-started guide** in the palette and in Settings.

#### Not done

- K.18 (modes) isn't built, so the second step teaches the reading options, not the modes (DD7). No Help menu yet.
  Not tried by hand.

### 2026-11-15 03:59; Ph1: 59/60 | U.7: Localization

#### Added

- Every string of the window comes from `app/locales/` (English and Arabic). **Settings, Appearance, Language**: the
  system's or a language named in its own script; Arabic mirrors the window. A pseudo-locale test fails on any text
  that skips the translations.

#### Not done

- The Arabic is not reviewed by a native speaker; vertical CJK text in ebooks.

### 2026-11-15 03:59; Ph1: 58/60 | U.4: High contrast, scaling and reduced motion

#### Added

- With Windows' high contrast on, every theme switches to its high-contrast side, and back when it goes off, without
  a restart (**Settings, Appearance, Follow the system's high contrast**, on by default). Text size and display
  scaling scale the window; with animation effects off, nothing moves.

#### Not done

- Not looked at by hand at 200 %.

### 2026-11-15 03:59; Ph1: 57/60 | U.2: Canvas accessibility on Windows

#### Added

- Screen readers see the pages: a Document named after the file, with the text in reading order (line, word and
  character reading, selection), each page, its links (which they can follow) and its annotation notes.

#### Not done

- Not tried with NVDA or Narrator. Pages far from the screen read as empty until scrolled near; text search,
  font attributes, tagged-PDF structure and form fields come later.

### 2026-11-15 03:59; Ph1: 56/60 | U.1: Keyboard access and focus

#### Added

- Every command is reachable from the keyboard, focus rings show where the keyboard is, and dialogs keep and give
  back the focus. A headless test reads a document with the keyboard alone.

#### Not done

- Read mode only; the other modes come with K.18. Not tried with a screen reader.

### 2026-11-15 03:13; Ph1: 55/60 | O.22: Print

#### Added

- **Print** (Ctrl+P): the Windows print dialog chooses the printer, the pages (all, the current one or ranges),
  copies, and duplex and colour. Pages print at the printer's resolution on a thread of their own; a strip at the
  window's foot shows the page being printed, with **Cancel printing**.
- **Settings, Printing:** **Page size on paper** (shrink to fit, fit to paper, actual size) and **Print annotations**.
  A landscape page turns to fit portrait paper.

#### Not done

- macOS and Linux printing (Phase 2). Scaling and annotations are settings, not in the dialog (DD6). Not tried on a
  printer yet.

### 2026-11-15 02:54; Ph1: 54/60 | F.17: Home page

#### Added

- **Home page**: what a pane with no file shows. **Continue reading** offers the file you read last, at its page;
  **Search recent files** finds pinned and recent files by name or folder; with more than one workspace, a chip per
  workspace switches to it.
- Pinned and recent files show each file's **cover** (its first page), drawn in the background and kept on disk, so
  the page appears at once and the covers fill in.

#### Not done

- Daily review comes with S.5. Paint time is not measured (K.24). Not tried by hand yet.

### 2026-11-15 02:48; Ph1: 53/60 | F.1: Find in document

#### Added

- **Find in document** (Ctrl+F): a bar above the pages that searches as you type, from your page on round the
  document, with **Match case**, **Whole words only** and **Match accents**. It counts the hits ("3 of 120").
- **Next / Previous result** (F3 / Shift+F3, Enter / Shift+Enter in the bar), round the ends. Every hit is
  highlighted on the pages, the current one in a stronger colour. Escape closes the bar and clears them.
- The **Results** tab, beside the Inspector, lists every hit with the words round it, in document order; pressing one goes there.
- A word split across lines by a hyphen is found whole; ligatures and Latin accents fold unless Match accents is on.

#### Not done

- Time to the first hit is not measured (K.24). The result list shows the first 500 hits. Jumps to hits are not in
  the back/forward history. Not tried by hand yet.

### 2026-11-15 02:26; Ph1: 45/60 to 52/60 | W.1 to W.7, W.14: Workspaces

#### Added

- **Workspaces**: named sets of tabs, each with its own panes and layout. The switcher at the left of the title bar
  shows the workspace's colour, icon and name; pressing it lists every workspace with its tab count (W.1).
- **New workspace** (Ctrl+Alt+N), **from all tabs**, or **Move tab to a new workspace**; a new workspace opens
  its edit dialog to be named (W.2).
- **Edit workspace**: name, icon, colour, Move up / Move down, and Delete, which moves its tabs to the workspace
  before it or closes them (W.3).
- **Move / Copy tab to workspace**, keeping the tab's page and view (W.4).
- **Go to workspace 1 to 9** (Ctrl+Alt+1 to 9), **Next / Previous workspace** (Ctrl+Alt+PageDown / PageUp) (W.5).
- Setting **A file open in another workspace**: switch there (default) or open here too (W.6).
- The window reopens on its last workspace (W.7). Workspaces not shown are library rows only: no document and no
  file handle (W.14).

#### Changed

- The session is saved per workspace, and at once on a switch.

#### Not done

- Warm cache and hibernation, tab thumbnails, the left rail, the tab context menu, drag to reorder or onto the
  switcher, and the §11.16 budgets (not measured).

### 2026-11-15 02:20; Ph1: 37/60 to 44/60 | V.1 to V.7, V.13: Split view

#### Added

- **Split view** (V.1 to V.7): up to four panes in a two-by-two grid, each with its own tabs, page canvas, page,
  thumbnails, outline, bookmarks and edits. **Split right** (`Ctrl+\`) and **Split down** (`Ctrl+Alt+\`) show the
  shown tab again at the same place; two panes of one file share its document, so it is read once.
- **Open files in a split** (Ctrl+Shift+O), and drop targets along the page area's four edges while a file is
  dragged over the window (V.2).
- **Move tab to the next / previous pane** (Ctrl+Alt+Right / Left), keeping its page and view (V.3).
- Divider drag to resize, double-click or Ctrl+Alt+0 to make panes the same size (V.4).
- The active pane has an accent border; **Go to pane 1 to 4** (Alt+1 to Alt+4), **Next / Previous pane** (F6 /
  Shift+F6). The tab strip, page button, sidebar panels and every command are the active pane's (V.5).
- **Close pane** (Ctrl+Alt+W, or the pane header's close button); the setting **Closing a pane** moves its tabs to
  the pane beside it (default) or closes them (V.6).
- The session keeps the layout and every pane's tabs (V.7; library migration `0005_panes.sql`).
- `--split a.pdf b.pdf` and `--split-same a.pdf`, at startup and through the running app (V.13).

#### Changed

- The shell's document area moved to `excellent-app::panes`; the old single-document wiring is gone.

#### Proposed (waiting for the maintainer)

- DD4: panes use Alt+1 to 4 because Ctrl+1 to 9 are the tabs' (R.12).

#### Not done

- Mouse drag of tabs between panes; "Open in split" in recent files and the library; V.1's 50 ms first frame is
  not measured.

### 2026-11-14 23:09; Ph1: 11/60 | K.25: Release engineering (first test installer)

#### Added

- `packaging/windows/build-msi.ps1` builds a per-user Windows installer from a release build:
  `ExcellentPDF-vYY-MM-DD.N-v0.1.0-windows-x64.msi` (DD3). It installs into `%LOCALAPPDATA%\Programs\ExcellentPDF`
  without administrator rights. It adds a Start menu shortcut and an entry in Settings > Apps, from where it
  uninstalls, keeping the reader's settings and library. It replaces an installed build of the same or an older
  version and refuses to replace a newer one. It ships the Visual C++ runtime next to the app, because the exe needs
  `msvcp140.dll` and `vcruntime140*.dll`. It opens the app when an install run from its window finishes.
- The script uses only what comes with Windows (the Windows Installer API and makecab, LZX:21), so nothing has to be
  installed to build it. Installers go to `dist/`, which git ignores.
- First test installer: `ExcellentPDF-v26-11-14.1-v0.1.0-windows-x64.msi`, 13.78 MB (budget 15 MB).

#### Checked

- An administrative install (`msiexec /a`) unpacked the four files, and the exe matched the build byte for byte.
- Not checked: a real install, upgrade and uninstall on this machine, the shortcut, and the launch at the end. These
  are left to the maintainer's test.

#### Not done (K.25 stays in progress)

- The installer is unsigned, has no dialogs (Windows shows only its progress window) and has no ARM64 build.
- Also still to do: the portable zip, updates, crash reports, and the release workflow.

### 2026-11-14 22:35; Ph1: 36/60 | B.6: Personal bookmarks

#### Added

- Personal bookmarks, kept in the library so every format has them: **Bookmark this place** (Ctrl+D) saves the
  selection or the top of the view at its zoom, in a dialog for its title, note, tags and colour. The sidebar's
  **Bookmarks** tab lists them in reading order with a search over titles, notes and tags; a press goes to the exact
  place. Edit and Delete are in each row's context menu, on F2 and Delete, and in the palette.
- A renamed or moved file finds its bookmarks again by its content the next time it is shown.

#### Changed

- K.15's fault-injection test cuts the log at each line's end, either side of it and inside each line, instead of at
  every byte: excellent-core's tests took 58 s instead of 1396 s on this machine.

### 2026-11-14 21:56; Ph1: 4/60, 5/60, 31/60 to 35/60 | K.14, K.15, B.1 to B.5: Editing the outline, with undo and autosave

#### Added

- The outline can be edited: **Add outline entry here** (Ctrl+B) at the selection or the top of the view, titled with
  the selected text; rename in place (F2); delete; move up and down (Alt+Up, Alt+Down), indent and outdent (Tab,
  Shift+Tab); point an entry at the place being read. All are in each row's context menu and the palette's Outline
  group. A document without an outline offers to add its first entry (B.1 to B.4).
- Ctrl+press and Shift+press mark several entries; delete and moves act on all of them as one step. Expanded and
  collapsed entries are written to the file with the next edit (B.5).
- Every edit is one step of undo: Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z). Each is written to an operation log on disk
  before it counts (K.14).
- Edited files save on their own 2 s after the last edit, when another tab is shown, and as the app quits; Ctrl+S
  saves now. A save writes beside the file, reads it back, then replaces it, so a failure leaves the file as it was.
  After a crash, the file offers its unsaved changes back: Restore or Discard (K.15).

#### Changed

- Escape in the outline gives the keyboard back to the pages, since Tab there now indents.
- Drag and drop of outline entries (B.3) waits for NSH: v0.5.0's tree has none.

### 2026-11-14 19:09; Ph1: 20/60 | R.9: Each tab keeps its view

#### Added

- The corpus's EPUB and XPS files carry links, and a test reads them (R.7).
- Left and Right (and H, L) go to the row of pages on that side, by the reading direction: Left goes on in a
  right-to-left document. Zoomed in past the width, they scroll sideways (R.3).
- Each tab keeps its zoom, layout (single, facing, book, strip or grid; paged or continuous; reading direction), view
  rotation and the place on the page, through tab switches, closing and reopening, and restarts (R.1, R.2, R.9). A
  page given alone, such as a launch's, still goes to that page.

#### Changed

- Settings for focus mode, the reading ruler and link protocols, and files from Explorer opening as tabs, were already
  built: R.7, R.12, R.19 and R.20 no longer list them as open.

#### Fixed

- A book in FB2, HTML, Markdown or MOBI kept the old page number when its text size changed, instead of the page
  where the reader's text went: the new layout was made only after the place was looked up (R.28).
- A second launch could stop the first instance listening for files when it wrote and closed before the first was
  ready for it; its message is now read (R.11, K.19).

### 2026-11-14 18:28; Ph1: 30/60 | R.28: Other formats through MuPDF

#### Added

- EPUB, MOBI, FB2, XPS, CBZ, SVG, HTML, Markdown and pictures open like PDFs: from the Open dialog, by a drop, and
  through "Set as default PDF viewer".
- Books are laid out to the window: each page is a screen tall and as wide as the window, up to a comfortable line
  length. Resizing the window lays the book out again, on the page the reader was on.
- Settings, Reading: "Book text size" (8 to 32 pt).
- A test corpus with one small file per format, made by `tests/corpus-src/make_formats.py`.

#### Changed

- DjVu is no longer offered: MuPDF can't read it (it is R.29's). Files are recognised by their contents in one place,
  `excellent-core::formats`.

### 2026-11-14 17:59; Phase labels for every feature

#### Changed

- Every feature is written as its label, `Ph<phase>: <n>/<count> | <TAG>: <Title>`, for example
  `Ph1: 22/60 | R.11: Opening files`, so the reader sees where it sits in its phase. The plan's 275 entries carry it
  in their headings; feature docs, `docs/features.toml` and the changelog headings use it too. Tags stay the permanent
  names (AGENTS.md section 4, plan §27.0).
- Centred dots between items are gone from the plan and the docs: semicolons separate them, and formulas use ×.

### 2026-11-14 17:32; Ph1: 22/60 | R.11: Opening files from outside the app

#### Added

- Documents dropped on the window open in new tabs. Each file is checked by its contents, not its name.
- Single instance: double-clicking a document, or starting the app with files, while it runs brings the running
  window to the front and opens them there. A file that already has a tab is shown in that tab.
- Command line: `--page N` shows the files at page N.
- "Set as default PDF viewer" in the command palette offers the app for its document types and opens Windows'
  Default apps settings at ExcellentPDF.

### 2026-11-14 17:12; Ph1: 21/60 | R.10: Recent and pinned files

#### Added

- With no tab open, the document panel lists pinned and recent files with the page each was left at. Pressing one
  opens it there; each row can be pinned or removed, and "Clear" empties the recent list but keeps pinned files.
- The tab list (Ctrl+Shift+A) has a "Recent files" section with the recent files that aren't open.
- A portable copy finds its files again after it moves to another folder or drive letter.

### 2026-11-14 16:45; Ph1: 10/60 | K.20: Library database; Ph1: 20/60 | R.9: Session restore; Ph1: 24/60 | R.13: Tab groups

#### Added

- The library database, `library.db`, next to `settings.toml`: SQLite in WAL mode with one writer thread, numbered
  migrations, and tests that upgrade every earlier version and survive a crash in the middle of a write.
- The app comes back as it was left: every tab at its page, pins, the shown tab, recently closed tabs and tab groups.
  The tabs are saved a second after they change and when the app quits.
- A tab whose file has gone shows greyed with a hollow ring; shown, its panel says where the file was and offers
  "Locate…".
- Tab groups: "Add tab to new group" in the command palette puts the shown tab in a named, coloured group. A chip in
  the strip names the group and collapses or expands it (a right click edits it); its tabs are underlined in its
  colour. The edit dialog renames, recolours (eight named colours), ungroups or closes the group. Moving a tab with
  Ctrl+Shift+PageUp / PageDown takes it into and out of groups. Collapsing hides tabs without closing them.

#### Changed

- A file given on the command line opens beside the restored tabs, or shows the restored tab of it.

### 2026-11-14 16:11; Ph1: 23/60 | R.12: Tabs

#### Added

- Every document opens in a tab of its own. The tab strip sits in the title bar: the shown tab takes the page's
  colour, a pin marks pinned tabs, a middle click or the close button closes a tab, and the arrow keys move between
  tabs. A + button and Ctrl+O open files through the system's dialog (several at once).
- Ctrl+W closes a tab, Ctrl+Shift+T reopens the last one closed (where it was, at the page it was left at),
  Ctrl+Tab and Ctrl+Shift+Tab go through the tabs, Ctrl+Shift+PageUp and PageDown move one. Pin, close others,
  close to the right and close duplicates are in the command palette.
- The tab list (Ctrl+Shift+A, or the chevron at the end of the strip): every open, duplicated and recently closed
  tab, searchable by name and folder.
- Settings, Tabs: whether Ctrl+Tab goes through the tabs in the order they were last used (the default) or along the
  strip (`tab_cycle_order`).
- The empty document panel has an "Open a document" button.

#### Changed

- Only the shown tab has a document open: switching tabs gives the one page canvas the other file. The outline,
  the thumbnails and file links follow the shown tab instead of the command line's file.
- The command palette's button in the title bar is narrower (280 points), to leave the tabs the room.

### 2026-11-14 15:07; Ph1: 26/60 | R.15: Reading themes, previewed live on the page

#### Added

- Reading themes: App theme (the default), Light, Sepia, Dark, OLED black, High contrast and Custom, each a paper and
  ink the pages are drawn in (`excellent-core::darkmode::themes`). Every built-in one passes WCAG AA, checked on a
  rendered tile of the corpus.
- The reading-theme bar: "Choose reading theme" in the palette, or the Reading theme button in Settings, puts the
  themes as swatches in the title bar; the arrow keys show each one on the open page, Enter keeps it, Escape goes
  back.
- Settings, Reading: "Reading theme" and "Custom theme" (paper and ink as `#rrggbb`, with their contrast in words).

#### Changed

- Choosing a reading theme while page colours are Original switches them to Smart.
- Whether paper is dark is judged by WCAG luminance, in the core, instead of the app's own rough sum.

### 2026-11-14 14:56: Local builds use the huacnlee/cargo fork

#### Changed

- `AGENTS.md` §7 gains "Building with the cargo fork": local builds use [huacnlee/cargo](https://github.com/huacnlee/cargo),
  whose shared, self-cleaning build directory keeps the disk from filling up. Agents check `cargo -V`, never clean
  or move the build directory, leave the fork's settings alone, and keep the repository buildable with the stock
  cargo.

### 2026-11-14 13:57; Ph1: 25/60 | R.14: Smart dark mode; pages in the theme's colours, pictures kept

#### Added

- **Page colours** (Settings, Reading): Original, Smart or Everything. Smart draws the pages in the theme's text and
  paper colours and leaves photos and artwork as they are; Everything recolours the pictures too. In a dark theme,
  links take the theme's accent colour.
- "Page colours: follow the theme" in the command palette, and Shift+I on the pages, switch between Original and
  Smart. The choice is kept.

#### Changed

- Selected text and the reading ruler's band show on dark pages: they are laid over them translucent.

### 2026-11-14 13:10; Ph1: 6/60 | K.16: Settings window, and settings.toml followed as you edit it

#### Added

- **Settings** (Ctrl+, or the command palette): appearance, the reading ruler, focus mode, auto-scroll speed, the
  links that open without asking, and every shortcut with its keys. Each change is saved as you make it.
- The Shortcuts section lists what in your `[shortcuts]` could not be bound, and opens `settings.toml` for editing.

#### Changed

- An edit of `settings.toml` takes effect within a second, keys included; no restart. A file broken by an edit is
  ignored (the old settings stay) until it is fixed.

### 2026-11-14 11:29; Ph1: 6/60 | K.16: Page keys, your own shortcuts and commands that need a document

#### Added

- Enhanced's plain keys work on the pages: `c` continuous, `[` `]` rotate, `g` go to page, `f` full screen, `j` `k`
  a line, Home and End, Backspace and Shift+Backspace back and forward, Enter and Shift+Enter a screen. They act only
  while the pages have the keyboard, so typing in a box is never hijacked.
- Your own shortcuts in `settings.toml`: `[shortcuts."go.to_page"]` with `keys = [...]` (anywhere) and
  `page_keys = [...]` (on the pages); `[]` removes a command's keys. Yours win over a default; keys that can't be
  bound and unknown command names are logged. Read at start, so a change needs a restart for now.

#### Changed

- The command palette leaves out page commands while no document is open.

### 2026-11-14 11:19; Ph1: 27/60 | R.19: Full screen and focus mode; Ph1: 28/60 | R.20: Reading ruler; Ph1: 29/60 | R.21: Auto-scroll

#### Added

- **Full screen** (`Ctrl+Shift+L`, or F11): the pages alone. The title bar comes back while the pointer is at the top
  edge; the sidebar and inspector come back as they were when you leave.
- **Focus mode**: the page around the paragraph you are reading is dimmed (or around the line, with
  `focus_scope = "line"`).
- **Reading ruler**: a band on the line you are reading. It follows the pointer and snaps to lines; `↓` and `↑` move
  it a line at a time, one column to its end before the next. **Reading ruler: dim the page around it** inverts it.
  Its look is set in `settings.toml` (`reading_ruler_height`, `_color`, `_opacity`, `_invert`).
- **Auto-scroll** (`Ctrl+Shift+H`): `Space` pauses, `+` and `−` change the speed, which is kept. Middle-click scrolls
  toward the pointer; another click stops it.
- `Esc` leaves full screen, focus mode, the ruler and auto-scroll.

#### Changed

- Plain keys (arrows, `Space`, `+`, `−`, `Esc`) act on the pages only while they have the keyboard focus, which they
  take when a document opens and when you press on them. Elsewhere, such as in the page list, the keys work as before.

### 2026-11-14 08:51; Ph1: 3/60 | K.13: Text selection and copy

#### Added

- Text can be selected: drag across it, or double-, triple- or quadruple-click for a word, a line or a paragraph, and
  keep dragging to grow by the same unit, across pages if need be. The pointer is an I-beam over text.
- **Copy selected text** (`Ctrl+C`, `Ctrl+Insert`) and **Select all text on this page** (`Ctrl+A`).
- `excellent-text`, the text layer every text feature will read pages through.

#### Changed

- Page text is read without MuPDF's glyph-tight boxes, so a selection is an even band per line. Copied text joins
  words hyphenated across lines, as MuPDF's own text does.

### 2026-11-14 08:21; Ph1: 31/60 | B.1: Outline (reading part)

#### Added

- The sidebar's **Outline** tab shows the document's table of contents, with the page each entry opens. A press or
  the arrow keys go to an entry's page, and Back returns; the entry you are reading is marked and kept in view as you
  turn pages. A document without an outline says so.

### 2026-11-14 08:10; Ph1: 16/60 | R.5: Thumbnails

#### Added

- The sidebar's **Pages** tab shows every page small, with its label; the page you are reading is ringed, and a press
  goes to a page. Thumbnails render in the background below the pages on screen and are kept on disk, so a file opened
  again shows them at once.

#### Changed

- The render workers and the open documents are shared by the whole app (`excellent-core::shared`), not owned by the
  page canvas.

### 2026-11-14 07:49; Ph1: 18/60 | R.7: Links

#### Added

- Links work: a click on a cross-reference jumps there (Back returns), and the cursor shows a hand over any link while
  the title bar says where it goes.
- Web and mail links open in their app. A link to a file, a program or another protocol asks first; a script link
  never runs. The protocols that open without asking are the `link_protocols` setting (`http,https,mailto`).

### 2026-11-14 07:12; Ph1: 1/60 | K.5: One parse per file; Ph1: 19/60 | R.8: History; Ph1: 17/60 | R.6: Page labels and go to page

#### Added

- **Go to page…** (Ctrl+G): type a page number or a label such as "iv"; the box says where Enter goes, or that there
  is no such page, as you type.
- The page you are reading shows in the title bar as "iv (4 of 210)"; press it to go to another page.
- **First page** (Ctrl+Home) and **Last page** (Ctrl+End).
- **Back** (Alt+←) and **Forward** (Alt+→), and the mouse's back and forward buttons, return to where you were before a
  jump.

#### Changed

- A file open in two views is parsed once and shares its display lists; a document can be put to sleep and woken
  again, never while it has unsaved changes.
- The view state (layout, rotation, page box, history) moved from the Windows canvas to `excellent-core::view`, so
  every platform shares it.

### 2026-11-14 06:37; Ph1: 2/60 | K.6: Layout and transforms; Ph1: 12/60 | R.1: Page layouts; Ph1: 13/60 | R.2: Fit modes; Ph1: 14/60 | R.3: Reading direction; Ph1: 15/60 | R.4: Rotation and page boxes

#### Added

- Layouts: **Single page** (Ctrl+6), **Facing pages** (Ctrl+7), **Book view** with the cover alone (Ctrl+8),
  **Horizontal strip** and **Page grid**, each continuous or one row at a time (**Toggle continuous scrolling**).
  Switching keeps your place.
- **Fit page** (Ctrl+0) and **Fit height** beside Fit width; a fit stays in force as the window changes size.
- **Toggle right-to-left reading (manga)**. A PDF that asks to be read right to left opens that way.
- **Rotate view clockwise** (Ctrl+Shift+=) and **anticlockwise** (Ctrl+Shift+−); the file is not changed.
- **Show media, crop, bleed, trim or art box**.
- In the horizontal strip the mouse wheel and Page Up/Down move along the strip.

#### Changed

- Opening a PDF reads each page's size from the page tree instead of loading every page: 10,000 pages lay out in under
  half a millisecond.
- Ink is kept on its page, so it stays in place through any layout, zoom, rotation or box. A stroke starts only on a
  page.

### 2026-11-14 00:38; Ph1: 6/60 | K.16: Settings file; the theme is kept between runs

#### Added

- `settings.toml` in the app's data folder (`%APPDATA%\com.excellentpdf.app`), or next to the exe in portable mode.
  Only changed values are written, unknown keys are kept, writes are atomic, and a file that doesn't parse is never
  overwritten.
- The theme and appearance you pick are kept and in place from the first frame of the next run.

#### Changed

- The theme picker takes the keyboard focus once it shows, and only then previews: the palette that opens it gave
  the focus back to what had it before, so arrow keys did nothing.

### 2026-11-14 00:32; Ph1: 7/60 | K.17: Themes with live preview

#### Added

- Seven theme families (Neutral, Aurora, Coral, Lagoon, Citrus, Grape, Forest), each light and dark and each with a
  high-contrast variant, made from one OKLCH palette and small files in `app/themes/`.
- **Choose theme** and **Choose appearance** in the command palette. Moving through either list applies the
  highlighted choice to the whole window at once; Enter keeps it, Escape goes back. No restart.
- Tokens for the page area, success, warning, the five mode accents and the annotation colours.
- Tests: every theme passes WCAG AA contrast (7:1 and 4.5:1 in high contrast) in both appearances, and a headless
  test drives the picker (preview, Escape, Enter).

#### Changed

- The command palette and the pickers share one list style (`list_dialog`).
- The page canvas draws the theme's page-area colour behind the pages.

#### Not done yet

- Density and UI scaling (K.17 step 3); keeping the choice between runs (K.16); a bundled UI font.

### 2026-11-13 22:46: Release profile and budget checks (T10)

#### Added

- `[profile.release]`: fat LTO, one codegen unit, stripped. The exe went from 39.7 MB to 36.6 MB.
- `tests/budgets.ps1`: measures the release build's installer size (the exe as an MSI-style LZX cabinet), cold start
  and time to the first page of a typical PDF, prints them against `tests/size/budgets.json` and
  `tests/perf/budgets.json`, and fails on a regression of more than 10 % over a recorded baseline.
- A `budgets` job in the Windows workflow that runs it on every pull request and writes the table to the job summary.
- The page canvas logs how long a document took to show its first page.

#### Changed

- The page canvas makes its Direct3D device on a background thread at startup, not on its first frame (about 50 ms
  sooner to the first page).
- Measured on the maintainer's PC: installer 11.79 MB (budget 15), cold start 632 ms (budget 300), first page 293 ms
  (budget 150). Plan K.24 and Phase 0 record the numbers and where the time goes.

### 2026-11-13 22:03: NSH upstream proposal for pen input (T9)

#### Added

- `dev notes/nsh_upstream_proposal_pen_input.md`: what ExcellentPDF asks of NSH. Pen pressure, tilt, twist, eraser
  and coalesced samples on pointer events; pointer input for drawn components; native components layered in NSH's
  composition tree, so NSH content can show over the page. Not filed yet.

### 2026-11-13 21:57: Pen ink on the Windows page canvas (T8)

#### Added

- Draw on a page with a pen. The stroke's width follows pressure, and every sample the pen makes is drawn, including
  the ones Windows coalesces between messages.
- On Windows 11 24H2 and later, Windows draws the newest pen points itself on its next screen refresh (a delegated ink
  trail), so ink keeps up with the pen tip without waiting for the app's frame.
- `excellent-ink::stroke`: strokes with a width per point, and a pressure-to-width curve.
- `crates/excellent-canvas-win/src/pointer.rs`: pen, touch and mouse samples, ported from SumatraPDF Enhanced's
  `PointerInput.h` with its tests.
- Each stroke logs how long its samples took to reach `Present`.

#### Changed

- The page canvas presents through DirectComposition (needed for the ink trail).
- The page canvas turns off Windows' pen tap and press-and-hold ripples.
- AGENTS.md: UI and interface work uses both the `ui-ux-pro-max` and `impeccable` skills.
- Plan §7.3 and K.8: the compositor ink trail and what T8 built.

### 2026-11-13 19:25: Windows page canvas with scroll and zoom (T7)

#### Added

- `crates/excellent-canvas`: the platform-neutral canvas model. The continuous page layout, the viewport (scroll, zoom
  about a point, fit width, SumatraPDF's zoom steps from 8.33 % to 6400 %) and the tiles it needs, with a cache that
  cancels tiles no longer on screen and evicts the least recently drawn over 64 MB.
- `crates/excellent-canvas-win`: the `excellent.page-canvas` NSH component on Windows. A child window drawn with
  Direct2D on a DXGI flip-model swap chain, showing tiles that the render workers make. While a new zoom renders,
  the previous zoom's tiles stay on screen. Wheel scrolls, Shift+wheel scrolls across, Ctrl+wheel zooms about the
  pointer.
- `excellent-core::render::Job::try_take` and `Job::when_done`: the UI thread is woken when a job finishes instead of
  waiting on it.
- The app opens the document named on the command line (`excellentpdf file.pdf`) in the Document panel, at fit width.
- Commands Zoom in (Ctrl+Plus), Zoom out (Ctrl+Minus), Actual size (Ctrl+1), Fit width (Ctrl+2), and Scroll one
  screen down or up (Page Down, Page Up), in the palette and as shortcuts.

#### Changed

- Plan §7.4: zoom buckets are the exact scale in 1/4096 steps, not coarse levels, so text is never stretched.
- Plan K.7 and K.8: what T7 built, what it did not, and the shortcuts it took.

#### Fixed

- The canvas missed its final size when Windows sent `WM_SIZE` from inside default message handling, leaving a black
  strip at the bottom.

### 2026-11-13 17:42: NSH app shell, command palette and keymap (T6); tab-list button noted

#### Added

- `crates/excellent-app` and `app/nsh.json`: the NSH 0.5.0 app (binary `excellentpdf`), with Rust components. A
  title bar of our own (sidebar toggle, app name, a command-palette button, inspector toggle; Windows draws the caption
  buttons) over a dock area with the document between a sidebar and an inspector. Each panel has an empty state that
  teaches one next step.
- The command table's first form (`excellent-app::commands`): the palette and the keyboard shortcuts are both built
  from it. Ctrl+K or Ctrl+Shift+P opens the palette, F12 toggles the sidebar, Ctrl+Alt+B the inspector.
- `app/locales/en.json`: all UI text, through NSH's translations.
- Headless tests (`crates/excellent-app/tests/shell.rs`): the three panels show, the shortcuts toggle the docks, and
  the palette lists every command and runs the one chosen.
- Plan §10.1 and R.12: the tab-list button at the right end of the tab strip (Vivaldi's), with search, open, inactive,
  duplicate and recently closed tabs. Reference image `dev notes/images/ui-reference-vivaldi-tab-list.png`.

#### Measured

- Cold start to a visible window: median 863 ms over 7 runs (release build), against the 300 ms budget. Most of it is
  inside NSH's startup before the first frame; to be profiled in T10.
- Release exe: 30 MB, before any release profile (T10).

### 2026-11-13 17:03; T5; Ph0: 4/13 | K.4: Render worker pool; build-once rule

#### Added

- `excellent-core::render::RenderPool`: worker threads (one per core but one) that build display lists, render tiles
  and extract text, highest priority first in the order of plan §7.4. Each job returns a `Job` you can wait on or
  cancel; dropping a job cancels it, and closing the pool cancels what is still waiting.
- `excellent_mupdf::Cookie`: cancels a running display-list build, tile render or text extraction from another
  thread. These three operations now take an optional cookie, and report a cancelled run as an error, never as a
  half-drawn result.
- Stress tests (`crates/excellent-core/tests/render_stress.rs`): four workers render 10,000 tiles from one document,
  each identical to the reference; cancelling a running render returns within 20 ms. New corpus file
  `heavy-vector.pdf`, a page slow enough to cancel part-way.
- The ASan job in `.github/workflows/fuzz.yml` runs the render stress tests too.
- AGENTS.md §7: write all the code for a task first, then build and test once, since Rust builds are slow.

#### Not yet checked

- The stress test has not run under ASan (needs nightly, as for T4).

### 2026-11-13 16:42; T4; Ph0: 3/13 | K.3: Safe MuPDF API

#### Added

- `crates/excellent-mupdf`: the safe Rust API over the shim. `Document` (open from a path or from bytes, page count,
  outline), `Page` (bounds, render, display list, text, annotations), `DisplayList` (render whole or a tile, from any
  thread), `Pixmap` (pixels, PNG), `StextPage`, `Annot`, `Writer` (write pages to a new PDF) and `MuError`, which
  carries MuPDF's message. Thread rules are in the types: `DisplayList` and `Pixmap` are `Send + Sync`, `Document` is
  `Send`, pages and annotations stay on their thread.
- Contexts: one base context with real locks per process, and one cloned per thread on first use.
- Typed geometry (`PagePoint`, `DevicePoint`, `PageRect`, `DeviceRect`, `Quad`, `Matrix`) and
  `excellent-core::transform`, the one place zoom, rotation and crop box become the page-to-device matrix.
- The PDF base-14 fonts and Charis SIL are compiled in, so documents without embedded fonts draw.
- The corpus test (`tests/corpus.rs`): page 1 of every corpus file renders to PNG and gives text; outline,
  annotations, threads, tiles, writing and errors are tested too. New corpus file `base14-outline-annot.pdf`.
- `tests/fuzz/` with the first fuzz target, `open_render`, and `.github/workflows/fuzz.yml`: the tests under
  AddressSanitizer and five minutes of fuzzing, on Linux, weekly and by hand.

#### Not yet checked

- ASan and fuzzing have not run: they need the nightly toolchain, which is not installed here, and the workflow has
  not run on GitHub.

### 2026-11-13 16:00: all of MuPDF on every OS; cross-platform CI

#### Added

- All of MuPDF on every target OS (`dev notes/dev_decisions.md` DD1): OCR (Tesseract, Leptonica), barcodes
  (zxing-cpp, zint), archives (libarchive, bzip2, liblzma), MuPDF's tools and SumatraPDF's OFD reader are now built,
  with per-OS fonts, signatures and JPEG XR for Windows, Linux, macOS, iOS and Android; libjpeg-turbo SIMD through
  NASM on x86.
- GitHub Actions (`dev notes/dev_decisions.md` DD2): a reusable build workflow and one workflow per platform; Windows
  builds, lints and tests on every push and pull request, the others run by hand until their phase.
- `dev notes/dev_decisions.md`, the record of the maintainer's working decisions.

### 2026-11-13 15:02: MuPDF bindings (T3)

#### Added

- `excellent-mupdf-sys`: `build.rs` compiles MuPDF and its libraries on Windows (MSVC); a C shim (`csrc/shim.c`)
  wraps context, open, page count, page load and bounds, rendering, and structured text in `fz_try` / `fz_catch`;
  committed bindgen bindings; smoke tests on `tests/corpus/zlib-manual.pdf`.

### 2026-11-13 14:17: vendored MuPDF (T2)

#### Added

- `third_party/`: MuPDF 1.28.5 from upstream SumatraPDF `b1a8e2257` with its 70 patches, SumatraPDF Enhanced's six
  annotation patches as `0088`–`0093`, the libraries MuPDF needs, and SumatraPDF's MuPDF glue; verified byte for byte
  against pristine 1.28.5 plus the 76 patches.

### 2026-11-13 14:07: workspace (T1)

#### Added

- Cargo workspace with the seventeen crates of plan §5.4, the repository layout of plan §17, workspace lints,
  `cargo-deny` configuration, budget files, the feature-spec and ADR templates, and ADRs for decisions D1–D10.
