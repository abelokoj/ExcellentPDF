# Build progress

Updated 2026-11-25 12:14 by `dev notes/build_progress.py` from the plan, `docs/features.toml`, `CHANGELOG.md`, git and the agent session transcripts. Only the Outstanding block is edited by hand.

A feature is **shipped** once the maintainer has accepted it, **in progress** from the first commit until then (built, tests passing, waiting for acceptance or with open gaps), and **not started** before that.

## Summary

| Phase | Total | Shipped | In progress | Not started |
| --- | --- | --- | --- | --- |
| Ph0 | 13 | 0 | 1 | 12 |
| Ph1 | 60 | 11 | 49 | 0 |
| Ph2 | 3 | 0 | 3 | 0 |
| Ph3 | 49 | 0 | 49 | 0 |
| Ph4 | 44 | 0 | 5 | 39 |
| Ph5 | 45 | 0 | 0 | 45 |
| Ph6 | 13 | 0 | 0 | 13 |
| Ph7 | 32 | 0 | 0 | 32 |
| Ph8 | 3 | 0 | 0 | 3 |
| Ph9 | 13 | 0 | 0 | 13 |
| **All** | **275** | **11** | **107** | **157** |

Work so far: 48:36 hours of agent time over 8 working days; 87 commits; 112 changelog entries.

## Outstanding

<!-- outstanding: kept by hand, the script leaves this block as it is -->

**Next:** Ph4: 6/44 | H.13: Handwriting straightening. Feature work is paused at the maintainer's request
(2026-11-25), after the app icon and installer release 4.

**Waiting for the maintainer:**
- Acceptance of the in-progress features: each becomes shipped once accepted.
- The flagged decisions in `dev notes/dev_decisions.md`, DD5 to DD28; among them the pack signing key (DD27) and the
  app icon's design (DD28).
- The open questions Q1 to Q7 (plan §21), Q1 (licence) among them.

**Over budget (last release measurement, not yet investigated):** cold start 2250 ms (limit 300 ms); opening a PDF
388 ms (limit 150 ms).

**Checks owed:**
- CI: confirm the macOS and GTK canvases compile with `faint_image` (H.8 onwards).
- Installer release 4 (`dist/ExcellentPDF-v26-11-25.4-v0.1.0-windows-x64.msi`) has not been test-installed.
- Phase 0's twelve "not started" features were built in tasks T1 to T10 but have no `docs/features.toml` entries;
  K.11 (iPad canvas), K.22 (sandboxed engine), K.23 (fuzzing) and K.26 (test corpus) need checking against the code.

**Deferred, with where they land:**
- U.8: input methods on the canvas wait for N.6, typed text blocks (DD26).
- K.21: the Settings page for packs comes in Phase 5.
- H.11: moving guides with two fingers waits for the canvas's touch gestures.

<!-- /outstanding -->

## Date log

Newest first. Time is agent working time (steps less than 15 minutes apart); the list is the changelog entries finished that day.

### 2026-11-25 (Wed): 6:05 hours; 20 commits

- NSH 0.7.0
- Ph3: 40/49 | H.3: Highlighter pen
- Ph3: 41/49 | H.5: Erasers
- Ph3: 42/49 | H.6: Lasso
- Ph3: 43/49 | H.7: Favourite pens bar and presets
- Fix: the main window shows again on NSH 0.7
- Ph3: 44/49 | H.8: Palm rejection and "finger draws"
- Ph3: 45/49 | H.9: Save ink as standard PDF
- Ph3: 46/49 | H.10: Shape recognition (hold to snap)
- Ph3: 47/49 | O.23: Screenshots and region snapshots
- Ph3: 48/49 | O.25: Attachments
- Ph3: 49/49 | U.8: Input methods on the canvas (deferred, DD26)
- Ph4: 1/44 | K.21: Pack manager (Phase 4 part)
- Ph4: 2/44 | R.23: Paper textures
- Ph4: 3/44 | H.4: Tape
- Ph4: 4/44 | H.11: Ruler, protractor and compass
- Ph4: 5/44 | H.12: Zoom writing pad
- App icon

### 2026-11-16 (Mon): 8:32 hours; 26 commits

- Ph3: 11/49 | W.8: One workspace per window
- Ph3: 12/49 | W.9: Search tabs across workspaces
- Ph3: 13/49 | W.11: Export and import
- Ph3: 18/49 | A.5: Sticky notes
- Ph3: 19/49 | A.6: Text boxes and typewriter
- Ph3: 20/49 | A.7: Callouts with leader lines
- Ph3: 21/49 | A.8: Lines and arrows
- Ph3: 22/49 | A.9: Rectangles and ellipses
- Ph3: 23/49 | A.10: Polygons, polylines and clouds
- Ph3: 24/49 | A.11: Margin notes and anchored note cards
- Ph3: 25/49 | A.12: Annotation inspector
- Ph3: 26/49 | A.13: Select, move, resize, copy, paste and delete
- Ph3: 27/49 | A.14: Annotation list with filter and search
- Ph3: 28/49 | A.15: Annotation recovery
- Ph3: 29/49 | A.16: Transfer annotations to another version
- Ph3: 30/49 | A.17: Semantic categories
- Ph3: 31/49 | A.18: Tags on annotations
- Ph3: 32/49 | A.19: Filters by color, tag, author and date
- Ph3: 33/49 | A.20: Stamps library with dynamic fields
- NSH 0.5.1
- Ph3: 34/49 | A.21: Comment threads with replies and resolve
- Ph3: 35/49 | A.22: Annotation layers by person or purpose
- Ph3: 36/49 | A.24: XFDF and FDF import and export
- Ph3: 37/49 | A.25: Flatten on export
- Ph3: 38/49 | A.28: W3C Web Annotation export
- Ph0: 13/13 | H.1: Ink capture and stroke pipeline
- Ph3: 39/49 | H.2: Pen types

### 2026-11-15 (Sun): 10:48 hours; 35 commits

- Ph1: 37/60 to 44/60 | V.1 to V.7, V.13: Split view
- Ph1: 45/60 to 52/60 | W.1 to W.7, W.14: Workspaces
- Ph1: 53/60 | F.1: Find in document
- Ph1: 54/60 | F.17: Home page
- Ph1: 55/60 | O.22: Print
- Ph1: 56/60 | U.1: Keyboard access and focus
- Ph1: 57/60 | U.2: Canvas accessibility on Windows
- Ph1: 58/60 | U.4: High contrast, scaling and reduced motion
- Ph1: 59/60 | U.7: Localization
- Ph1: 60/60 | U.9: First-run guide and tips
- Ph1: 58/60 | U.4: High contrast, scaling and reduced motion (fixes)
- Ph1: 8/60 | K.18: Modes and toolbars
- Ph2: 2/3 | K.10: Page canvas on Linux (shared canvas core)
- Ph2: 2/3 | K.10: Page canvas on Linux (GTK widget)
- Ph2: 1/3 | K.9: Page canvas on macOS (AppKit view)
- Ph2: 3/3 | U.3: Canvas accessibility on macOS and Linux
- Ph1: 55/60 | O.22: Printing on macOS and Linux
- Ph3: 1/49 | K.28: Annotation anchors
- Ph1: 4/60 | K.14: Annotation model
- Ph3: 2/49 | R.17: Mini-map with note ticks
- Ph3: 3/49 | R.25: Citation and equation hover
- Ph3: 4/49 | B.7: Bold, italic and color
- Ph3: 5/49 | B.9: Check destinations
- Ph3: 6/49 | B.10: Sort, promote and demote between outline and bookmarks
- Ph3: 7/49 | V.8: Synchronized scrolling
- Ph3: 8/49 | V.9: Open a link in the other pane
- Ph3: 9/49 | V.10: Swap, flip and maximize
- Ph3: 10/49 | V.11: Pop out to a window
- Ph3: 14/49 | A.1: Text markup: highlight, underline, squiggly, strikeout
- Ph3: 15/49 | A.2: Replace and insert text
- Ph3: 16/49 | A.3: Annotation colors
- Ph3: 17/49 | A.4: Notes on highlights

### 2026-11-14 (Sat): 12:50 hours; 5 commits

- Ph1: 7/60 | K.17: Themes with live preview
- Ph1: 6/60 | K.16: Settings file; the theme is kept between runs
- Ph1: 2/60 | K.6: Layout and transforms; Ph1: 12/60 | R.1: Page layouts; Ph1: 13/60 | R.2: Fit modes; Ph1: 14/60 | R.3: Reading direction; Ph1: 15/60 | R.4: Rotation and page boxes
- Ph1: 1/60 | K.5: One parse per file; Ph1: 19/60 | R.8: History; Ph1: 17/60 | R.6: Page labels and go to page
- Ph1: 18/60 | R.7: Links
- Ph1: 16/60 | R.5: Thumbnails
- Ph1: 31/60 | B.1: Outline (reading part)
- Ph1: 3/60 | K.13: Text selection and copy
- Ph1: 27/60 | R.19: Full screen and focus mode; Ph1: 28/60 | R.20: Reading ruler; Ph1: 29/60 | R.21: Auto-scroll
- Ph1: 6/60 | K.16: Page keys, your own shortcuts and commands that need a document
- Ph1: 6/60 | K.16: Settings window, and settings.toml followed as you edit it
- Ph1: 25/60 | R.14: Smart dark mode; pages in the theme's colours, pictures kept
- Local builds use the huacnlee/cargo fork
- Ph1: 26/60 | R.15: Reading themes, previewed live on the page
- Ph1: 23/60 | R.12: Tabs
- Ph1: 10/60 | K.20: Library database; Ph1: 20/60 | R.9: Session restore; Ph1: 24/60 | R.13: Tab groups
- Ph1: 21/60 | R.10: Recent and pinned files
- Ph1: 22/60 | R.11: Opening files from outside the app
- Phase labels for every feature
- Ph1: 30/60 | R.28: Other formats through MuPDF
- Ph1: 20/60 | R.9: Each tab keeps its view
- Ph1: 4/60, 5/60, 31/60 to 35/60 | K.14, K.15, B.1 to B.5: Editing the outline, with undo and autosave
- Ph1: 36/60 | B.6: Personal bookmarks
- Ph1: 11/60 | K.25: Release engineering (first test installer)

### 2026-11-13 (Fri): 6:34 hours

- workspace (T1)
- vendored MuPDF (T2)
- MuPDF bindings (T3)
- all of MuPDF on every OS; cross-platform CI
- T4; Ph0: 3/13 | K.3: Safe MuPDF API
- T5; Ph0: 4/13 | K.4: Render worker pool; build-once rule
- NSH app shell, command palette and keymap (T6); tab-list button noted
- Windows page canvas with scroll and zoom (T7)
- Pen ink on the Windows page canvas (T8)
- NSH upstream proposal for pen input (T9)
- Release profile and budget checks (T10)

### 2026-11-12 (Thu): 1:04 hours

- (planning and setup; no changelog entries)

### 2026-11-10 (Tue): 0:10 hours

- (planning and setup; no changelog entries)

### 2026-11-09 (Mon): 2:33 hours

- (planning and setup; no changelog entries)

### 2026-10-18 (Sun): 1 commits

- (planning and setup; no changelog entries)

## Features by phase

### Phase 0: 0 shipped; 1 in progress; 12 not started

**In progress (1)**

- Ph0: 13/13 | H.1: Ink capture and stroke pipeline (open: latency not measured on the reference device; synthetic session fixture only; mean width until H.9 (DD24))

**Not started (12)**

- Ph0: 1/13 | K.1: Repository and workspace skeleton
- Ph0: 2/13 | K.2: MuPDF build (`excellent-mupdf-sys`)
- Ph0: 3/13 | K.3: Safe Rust API (`excellent-mupdf`)
- Ph0: 4/13 | K.4: Render workers and threading
- Ph0: 5/13 | K.7: Tile cache and render scheduler
- Ph0: 6/13 | K.8: Page canvas on Windows
- Ph0: 7/13 | K.11: Page canvas on iPad
- Ph0: 8/13 | K.22: Out-of-process engine and sandbox
- Ph0: 9/13 | K.23: Security limits and fuzzing
- Ph0: 10/13 | K.24: Performance instrumentation and budgets
- Ph0: 11/13 | K.26: Test corpus, golden images and interop suite
- Ph0: 12/13 | K.27: Feature checklist in CI

### Phase 1: 11 shipped; 49 in progress; 0 not started

**Shipped (11)**

- Ph1: 1/60 | K.5: Document and View model, document registry (open: View serialization (R.9); annotation model and operation log (K.13, K.14); tiles shared between views)
- Ph1: 2/60 | K.6: Layout and coordinate transforms (open: non-PDF formats load each page for its size)
- Ph1: 8/60 | K.18: Modes and toolbars (open: Write and Study have no tools until the H.* and S.* tags; text labels, no icons yet (DD8); ink is not gated by mode until H.1; not tried by hand)
- Ph1: 10/60 | K.20: Library database (`excellent-store`) (open: tables of documents, tags, collections, bookmarks, cards and notes, and FTS5, come with their features' migrations; startup cost not measured (K.24); its need K.1 (task T1) is not marked here)
- Ph1: 15/60 | R.4: Rotation and crop-box view (open: selection uses the same placement when it is built)
- Ph1: 17/60 | R.6: Page labels and go to page (open: the page indicator moves from the title bar to the status bar when it exists; label editing is O.1)
- Ph1: 19/60 | R.8: Back and forward history (open: search pushes when it is built)
- Ph1: 56/60 | U.1: Keyboard access and focus (open: not tried by hand with a screen reader)
- Ph1: 58/60 | U.4: High contrast, color-blind-safe colors, scaling and reduced motion (open: not looked at by hand at 200 %; colour-blind-safe annotation palettes come with the annotation tags)
- Ph1: 59/60 | U.7: Localization (open: Arabic not reviewed by a native speaker; vertical CJK text in ebooks)
- Ph1: 60/60 | U.9: First-run guide and tips (open: no Help menu yet; not tried by hand)

**In progress (49)**

- Ph1: 3/60 | K.13: Text layer (`excellent-text`) and selection (open: reading order and ICU4X normalization and words (steps 2, 3); rectangle selection; edge scrolling; select all of the document; text-quality corpus (step 5))
- Ph1: 4/60 | K.14: Annotation model and operation log (open: undo after reopening; author in records; unknown annotation types are left alone but not yet modelled)
- Ph1: 5/60 | K.15: Saving, autosave, recovery and external changes (open: watching for outside changes and keep both (step 5); saving at sign-out and shutdown (step 6); fault injection; qpdf --check)
- Ph1: 6/60 | K.16: Settings, command table and shortcuts (open: menus and toolbars (and a Settings entry in them); the mode column; changing a key inside the Settings window; Enhanced import; needs K.5)
- Ph1: 7/60 | K.17: Design tokens and themes (open: step 3: density and UI scaling)
- Ph1: 9/60 | K.19: Platform services (open: single instance and file associations only (for R.11); dialogs, print, clipboard, speech, key store, notifications, memory pressure, the traits and a fake for tests come with the features that need them)
- Ph1: 11/60 | K.25: Release engineering: installers, updates, crash reports (open: the per-user x64 MSI builds (unsigned, no dialogs); signing, ARM64, the portable zip, updates, crash reports and the release workflow)
- Ph1: 12/60 | R.1: Page layouts (open: toolbar toggle and menus)
- Ph1: 13/60 | R.2: Zoom and fit modes (open: fit content; presets; custom zoom dialog; pinch (U.3))
- Ph1: 14/60 | R.3: Reading direction: right-to-left and manga (open: swipe follows the direction (U.3); EPUB page progression)
- Ph1: 16/60 | R.5: Thumbnails (open: measure 60 fps on 5,000 pages (K.24); follow the reader's page; trim the cache (K.20); the view's rotation)
- Ph1: 18/60 | R.7: Links (open: page preview on hover (needs R.5))
- Ph1: 20/60 | R.9: Session restore (open: quit and restart tried by hand; Ctrl+Tab's recent order; windows, panes and workspaces (W.1, W.4))
- Ph1: 21/60 | R.10: Recent and pinned files (open: tried by hand; File menu, jump list and Dock menu (with R.11))
- Ph1: 22/60 | R.11: Opening files: drag and drop, file associations, single instance, command line (open: tried by hand; registering on install (no installer yet); pane-edge drops (V.2); --zoom, --search, --split (V.13), --forward-search (R.27); macOS and Linux file types; its need K.19 is in progress)
- Ph1: 23/60 | R.12: Tabs (open: 50 tabs tried by hand; mouse drag to reorder; tab context menu (airspace: popups fall behind the canvas); the other workspaces' tabs listed with W.9)
- Ph1: 24/60 | R.13: Named tab groups (open: tried by hand; mouse drag in and out of groups (with R.12's drag); saved named sets as Enhanced's, with workspaces (W.1); its need R.12 is in progress)
- Ph1: 25/60 | R.14: Smart dark mode (open: object-level OKLab device and Enhanced's PdfDarkMode*_ut.cpp tests; golden images against Enhanced on the corpus; thumbnails not recoloured; speed not measured (K.24); its need K.7 is not marked here)
- Ph1: 26/60 | R.15: Reading themes (open: per-document override (needs K.20); reflowable formats through MuPDF's user stylesheet (step 2); live preview tried by hand; its need R.14 is in progress)
- Ph1: 27/60 | R.19: Full-screen reading and focus mode (open: try Esc and full screen by hand; follow a text caret once one exists)
- Ph1: 28/60 | R.20: Reading ruler (reading bar) (open: try by hand on a two-column paper)
- Ph1: 29/60 | R.21: Auto-scroll (open: measure smoothness at 60 and 120 Hz and idle CPU while paused (K.24); its need K.7 is not marked here)
- Ph1: 30/60 | R.28: Other formats through MuPDF (open: tried by hand; two views of one book share its layout (V.1))
- Ph1: 31/60 | B.1: Add an outline entry (open: an added entry checked in Acrobat and pdf.js (by hand); hover preview; 10,000-entry check (K.24))
- Ph1: 32/60 | B.2: Rename and delete (open: promote the children one level before deleting)
- Ph1: 33/60 | B.3: Move entries (open: drag and drop (NSH v0.5.0 has no element drag); 500-entry subtree in a 10,000-entry outline (needs a corpus file))
- Ph1: 34/60 | B.4: Set destination to the current view (open: the inspector shows the entry's page and zoom)
- Ph1: 35/60 | B.5: Multi-select, undo and open state (open: multi-entry move undone in one step, checked end to end by hand; restyle; open state saved without another edit)
- Ph1: 36/60 | B.6: Personal bookmarks (open: survives a re-typeset version (needs K.28, Phase 3); edited-then-renamed file; checked in the app by hand)
- Ph1: 37/60 | V.1: Split the current tab (open: tried by hand; the 50 ms first frame is not measured (K.24); a split of a reflowed book shares its layout)
- Ph1: 38/60 | V.2: Open another file in a pane (open: tried by hand; “Open in split” in the recent-files panel and the library (F.1))
- Ph1: 39/60 | V.3: Tab groups per pane (open: tried by hand; dragging tabs between panes with the mouse (with R.12's drag))
- Ph1: 40/60 | V.4: Pane layout (open: tried by hand)
- Ph1: 41/60 | V.5: Active pane and focus (open: tried by hand; Alt+1 to 4 instead of the plan's Ctrl+1 to 4 waits for approval (dev_decisions DD4))
- Ph1: 42/60 | V.6: Closing panes (open: tried by hand; no prompt on close (edits are saved on their own, K.15))
- Ph1: 43/60 | V.7: Saving split layouts (open: quit and restart tried by hand; workspaces (W.1))
- Ph1: 44/60 | V.13: Split from the command line (open: tried by hand)
- Ph1: 45/60 | W.1: Workspace switcher (open: tried by hand; left rail; dirty dot (autosave); tab strip tint)
- Ph1: 46/60 | W.2: Create a workspace (open: tried by hand; multi-select of tabs (with R.12))
- Ph1: 47/60 | W.3: Edit and delete workspaces (open: tried by hand; reorder by drag)
- Ph1: 48/60 | W.4: Move or copy tabs between workspaces (open: tried by hand; tab context menu and drag onto the switcher)
- Ph1: 49/60 | W.5: Workspace shortcuts (open: tried by hand)
- Ph1: 50/60 | W.6: Opening a file that is open elsewhere (open: tried by hand; matching by file identity (K.5))
- Ph1: 51/60 | W.7: Restore per window (open: tried by hand; two windows restoring two workspaces comes with W.8)
- Ph1: 52/60 | W.14: Lazy restore and hibernation (open: warm cache and hibernation; tab thumbnails; budgets and the leak test (K.24))
- Ph1: 53/60 | F.1: Find in document (open: time to first hit not measured (K.24); result list capped at 500; hits not in history; accent folding Latin only; not tried by hand)
- Ph1: 54/60 | F.17: Home page (open: daily review (S.5); cold-start paint not measured (K.24); cover folder not trimmed; not tried by hand)
- Ph1: 55/60 | O.22: Print (open: not tried on a printer on any desktop; Linux names the job after the spool file; no Current page on Linux or macOS; scaling and annotations are settings, not in the dialog (DD6); view rotation not applied; not tried on a printer)
- Ph1: 57/60 | U.2: Canvas accessibility on Windows (open: not tried with NVDA or Narrator; far pages read as empty; FindText and text attributes; tagged-PDF structure; form fields)

### Phase 2: 0 shipped; 3 in progress; 0 not started

**In progress (3)**

- Ph2: 1/3 | K.9: Page canvas on macOS (open: not tried on a Mac; pen tilt and coalesced tablet points; Metal host if K.24 asks (DD10))
- Ph2: 2/3 | K.10: Page canvas on Linux (open: not tried on Wayland or X11; pen tilt and GestureStylus backlog; touch gestures)
- Ph2: 3/3 | U.3: Canvas accessibility on macOS, Linux and tablets (open: not tried with VoiceOver or Orca; pages, links and notes as elements; sentences; UTF-16 offsets outside the BMP on macOS)

### Phase 3: 0 shipped; 49 in progress; 0 not started

**In progress (49)**

- Ph3: 1/49 | K.28: Annotation anchors (open: anchors are stored with each annotation (K.14 step 1); still open: the sidecar for non-PDF formats, and re-anchoring on open)
- Ph3: 2/49 | R.17: Mini-map with note ticks (open: not tried by hand; no setting to hide it; the strip is 12 px, under the 24 px target (keyboard equivalents exist))
- Ph3: 3/49 | R.25: Citation and equation hover (open: not tried by hand; no wheel zoom in the preview; no link-zoom path; text cache not keyed by page revision)
- Ph3: 4/49 | B.7: Bold, italic and color (open: not opened in Acrobat or tried by hand; fixed set of five swatches)
- Ph3: 5/49 | B.9: Check destinations (open: not tried by hand; the nearest heading is the one with the entry's title until B.8; deleted-page destinations resolve to page 1 in MuPDF and aren't flagged)
- Ph3: 6/49 | B.10: Sort, promote and demote between outline and bookmarks (open: not tried by hand; a named destination's own point is not read)
- Ph3: 7/49 | V.8: Synchronized scrolling (open: not tried by hand; page pairs are not kept per file)
- Ph3: 8/49 | V.9: Open a link in the other pane (open: not tried by hand; a search result opens at its page, not the hit)
- Ph3: 9/49 | V.10: Swap, flip and maximize (open: not tried by hand; a double-click on a header button may also count as a header double-click)
- Ph3: 10/49 | V.11: Pop out to a window (open: not tried by hand; docking is a command, a button or closing the window, not a drag; tab only, not a group)
- Ph3: 11/49 | W.8: One workspace per window (open: not tried by hand; quitting with workspace windows open not checked to keep them for the next start)
- Ph3: 12/49 | W.9: Search tabs across workspaces (open: not tried by hand; 5,000 tabs filtering within 16 ms not measured (NSH's command list does the filtering); another window's own workspace is listed as saved, up to a second behind)
- Ph3: 13/49 | W.11: Export and import (open: not tried by hand; the chosen root is the folder the file is saved in; closed tabs are not exported)
- Ph3: 14/49 | A.1: Text markup: highlight, underline, squiggly, strikeout (open: not tried by hand; not checked in Acrobat, PDF-XChange or pdf.js (K.26); the bar isn't exposed to screen readers (its commands are); the author is the system's user name)
- Ph3: 15/49 | A.2: Replace and insert text (open: not tried by hand; not checked in Acrobat (its Done when); a replacement across pages proposes only its last page's part; the caret is drawn upright on rotated text)
- Ph3: 16/49 | A.3: Annotation colors (open: not tried by hand; needs K.17 (in progress); the dialog opens on the highlighter from the command, on the bar's current tool from the bar)
- Ph3: 17/49 | A.4: Notes on highlights (open: not tried by hand; not opened in Acrobat or another reader yet; the editor is the inspector (the separate popover is not built); an undo while a note is open is not shown in it until it is opened again)
- Ph3: 18/49 | A.5: Sticky notes (open: not tried by hand; not opened in Acrobat yet; the page shows no sign while a note is being placed (the inspector says so); a sticky note can't be dragged yet (A.13))
- Ph3: 19/49 | A.6: Text boxes and typewriter (open: not tried by hand; not opened in Acrobat; text is typed in the inspector, not on the page, until U.8 gives the canvas the platform input path; boxes are fitted from average character widths, not the fonts' metrics; no background colour control or settings for new boxes' style yet; can't be moved or resized until A.13)
- Ph3: 20/49 | A.7: Callouts with leader lines (open: not tried by hand; not opened in Acrobat; the box and the tip have no drag handles yet (step 2 waits for A.13's selection handles); the leader line is written from its tip alone (MuPDF draws the knee-less line to the box's edge), so a three-point line from another app keeps only its tip when the box is edited)
- Ph3: 21/49 | A.8: Lines and arrows (open: not tried by hand; not opened in Acrobat; Shift snapping reads the key on Windows only (GTK and macOS draw unsnapped until their hosts read it); no width control yet (A.12's inspector); lines can't be moved or their ends dragged until A.13)
- Ph3: 22/49 | A.9: Rectangles and ellipses (open: not tried by hand; not opened in Acrobat; Shift makes squares and circles on Windows only (GTK and macOS until their hosts read the key); shapes can't be moved or resized until A.13)
- Ph3: 23/49 | A.10: Polygons, polylines and clouds (open: not tried by hand; clouds not opened in Acrobat; Shift snapping on Windows only; shapes can't be moved or their corners dragged until A.13)
- Ph3: 24/49 | A.11: Margin notes and anchored note cards (open: not tried by hand; the column sits beside the pages, not over them (DD11); Expand margins (N.3) is Phase 4's; a card's height is estimated from its text, so a long line can be cut)
- Ph3: 25/49 | A.12: Annotation inspector (open: not tried by hand; category, tags and layer rows wait for A.17, A.18 and A.22; multi-selection waits for A.13's selection; dates show in UTC)
- Ph3: 26/49 | A.13: Select, move, resize, copy, paste and delete (open: not tried by hand; the annotation clipboard is the app's own (no image fallback, not shared between two running copies; DD12); Ctrl-click and Ctrl-marquee read Ctrl on Windows only; inspector edits on several selected at once (A.12 item 2) not built)
- Ph3: 27/49 | A.14: Annotation list with filter and search (open: not tried by hand; the list is a tab of the right dock beside the inspector, not of the left sidebar, and rows show a colour bar instead of a type icon (DD13); the whole document is read again after every edit)
- Ph3: 28/49 | A.15: Annotation recovery (open: not tried by hand; Keep and Discard are the bar's buttons only, with no shortcut or palette command (the bar shows once, after a crash); a file changed on disk since its edits were logged still drops them (K.15 step 5))
- Ph3: 29/49 | A.16: Transfer annotations to another version (open: not tried by hand, and not on a real preprint and its published version (the test typesets one); the review is a dialog with counts and the first five not found, not a list to pick from (DD14); annotations that mark no text keep their page and place; thousands of highlights in a long book take seconds)
- Ph3: 30/49 | A.17: Semantic categories (open: not tried by hand; picked in the inspector, not the selection toolbar (DD15); icons are named (Lucide) but drawn as colour dots until the app has an icon set; export by category and the A.26 reports wait for A.26)
- Ph3: 31/49 | A.18: Tags on annotations (open: not tried by hand; the library-wide tag: search has its query (excellent_store::annotation_tags::find) but no search box until F.3 and F.4 (Phase 5, DD16); the index is written after each save by reading every page's annotations)
- Ph3: 32/49 | A.19: Filters by color, tag, author and date (open: not tried by hand; show only matching hides annotations in the page views and thumbnails, not in printing or export (DD17), and takes effect once the Annotations tab has read the file)
- Ph3: 33/49 | A.20: Stamps library with dynamic fields (open: not tried by hand and not opened in Acrobat; dynamic stamps write the time in UTC; image stamps are kept as images in the library (DD18))
- Ph3: 34/49 | A.21: Comment threads with replies and resolve (open: not tried by hand and not opened in Acrobat; replies are not carried over by A.16's transfer; Reply has no shortcut or palette command (DD19))
- Ph3: 35/49 | A.22: Annotation layers by person or purpose (open: not tried by hand and not opened in Acrobat; lock keeps visibility only (DD20); layers can't be renamed or deleted)
- Ph3: 36/49 | A.24: XFDF and FDF import and export (open: not tried by hand and not opened in Acrobat; XFDF loses a note's open state and custom stamp art (DD21))
- Ph3: 37/49 | A.25: Flatten on export (open: not tried by hand; form fields stay fillable (DD22))
- Ph3: 38/49 | A.28: W3C Web Annotation export (open: not tried by hand; no JSON-LD processor in the tests (DD23))
- Ph3: 39/49 | H.2: Pen types (open: golden images from the synthetic session; texture and blend not in the saved appearance until H.9; pen per pane, one colour and size until H.7 (DD25))
- Ph3: 40/49 | H.3: Highlighter pen (open: no snap to text lines (K.13); one colour until H.7 (DD25))
- Ph3: 41/49 | H.5: Erasers (open: ink goes on pen-up, not live; area pieces stay in one annotation until H.6; fixed eraser size until H.7)
- Ph3: 42/49 | H.6: Lasso (open: quarter turns only, no free-turning handle; copy as image (O.23) and convert to text (H.24) not built; Enhanced has no lasso tests to port)
- Ph3: 43/49 | H.7: Favourite pens bar and presets (open: Y.2 sync is Phase 9 (export and import until then); palette colours only)
- Ph3: 44/49 | H.8: Palm rejection and "finger draws" (open: synthetic session (DD24); Windows only, macOS and Linux hosts not wired; real hardware untested)
- Ph3: 45/49 | H.9: Save ink as standard PDF (open: Acrobat, PDF-XChange and pdf.js not checked (K.26); raw samples key not built; pencil grain not in the appearance; erased pieces lose widths)
- Ph3: 46/49 | H.10: Shape recognition (hold to snap) (open: synthetic shape set (DD24); no preview while the pen is held; shapes saved as ink)
- Ph3: 47/49 | O.23: Screenshots and region snapshots (open: make a card waits for S.1; annotations always included)
- Ph3: 48/49 | O.25: Attachments (open: page attachment annotations not added; no removal; /Kids trees get a new leaf)
- Ph3: 49/49 | U.8: Input methods on the canvas (open: the canvas's own IME path waits for N.6, the first in-canvas editor (DD26); all text entry today is NSH's, which composes on every desktop)

### Phase 4: 0 shipped; 5 in progress; 39 not started

**In progress (5)**

- Ph4: 1/44 | K.21: Pack manager (open: trusted key is the maintainer's to make (DD27); Settings page, index address and signing tools in Phase 5)
- Ph4: 2/44 | R.23: Paper textures (open: scroll frame time with the grain not measured; one texture)
- Ph4: 3/44 | H.4: Tape (open: no reveal or cover all; model in excellent-features rather than excellent-ink)
- Ph4: 4/44 | H.11: Ruler, protractor and compass (open: two-finger moves wait for canvas touch gestures; one guide, not kept across runs)
- Ph4: 5/44 | H.12: Zoom writing pad (open: the pad shows only this session's writing in the box; no manual box move or new-line key; page turn at the foot is the reader's)

**Not started (39)**

- Ph4: 6/44 | H.13: Handwriting straightening
- Ph4: 7/44 | H.14: Layers
- Ph4: 8/44 | H.15: Bulk restyle
- Ph4: 9/44 | H.16: Fill and gradients
- Ph4: 10/44 | H.17: Eyedropper
- Ph4: 11/44 | H.18: Saved palettes
- Ph4: 12/44 | H.22: Stroke replay
- Ph4: 13/44 | H.26: Convert shapes to vectors
- Ph4: 14/44 | N.1: Blank notebooks with procedural paper
- Ph4: 15/44 | N.2: Insert pages anywhere
- Ph4: 16/44 | N.3: Expand margins
- Ph4: 17/44 | N.4: Notebook format (`.expdf`) and export to PDF
- Ph4: 18/44 | N.5: Infinite canvas pages and whiteboards
- Ph4: 19/44 | N.6: Typed text blocks
- Ph4: 20/44 | N.7: Images, stickers and sticky notes on pages
- Ph4: 21/44 | N.8: Covers and sections
- Ph4: 22/44 | N.9: Links between pages
- Ph4: 23/44 | N.10: Quick note from a global shortcut
- Ph4: 24/44 | N.14: Markdown editing with math and Mermaid
- Ph4: 25/44 | L.1: Record audio while writing
- Ph4: 26/44 | L.3: Import slides and condense to notes
- Ph4: 27/44 | S.1: Flashcards from highlights, notes and lasso selections
- Ph4: 28/44 | S.2: Image occlusion
- Ph4: 29/44 | S.3: Cloze deletions
- Ph4: 30/44 | S.4: Spaced repetition (FSRS)
- Ph4: 31/44 | S.5: Daily review on the home page
- Ph4: 32/44 | S.6: Offline dictionary
- Ph4: 33/44 | S.7: Vocabulary practice
- Ph4: 34/44 | S.8: Study export
- Ph4: 35/44 | S.10: Exam mode
- Ph4: 36/44 | S.11: Anki import and export
- Ph4: 37/44 | S.12: Goals, timer, statistics and streaks
- Ph4: 38/44 | G.1: Present mode
- Ph4: 39/44 | G.2: Laser with trails; G.3: Spotlight; G.4: Magnifier
- Ph4: 40/44 | G.5: Pen on slides and fading ink
- Ph4: 41/44 | G.6: Whiteboard page
- Ph4: 42/44 | G.7: Curtain reveal
- Ph4: 43/44 | G.8: Presenter view
- Ph4: 44/44 | U.5: Read aloud

### Phase 5: 0 shipped; 0 in progress; 45 not started

**Not started (45)**

- Ph5: 1/45 | R.16: Reflow (Liquid Mode)
- Ph5: 2/45 | R.22: Bionic reading
- Ph5: 3/45 | R.24: Two-column auto-detect
- Ph5: 4/45 | R.27: SyncTeX forward and inverse search
- Ph5: 5/45 | R.29: DjVu, CHM and CBR
- Ph5: 6/45 | R.30: Open from a URL with progressive loading
- Ph5: 7/45 | B.8: Generate an outline
- Ph5: 8/45 | B.11: Import and export
- Ph5: 9/45 | B.13: Custom outline for non-PDF formats
- Ph5: 10/45 | V.12: Tiling selected tabs
- Ph5: 11/45 | W.10: Workspaces and library collections
- Ph5: 12/45 | W.12: Templates and rules
- Ph5: 13/45 | A.26: Annotation reports
- Ph5: 14/45 | A.27: Annotation history timeline
- Ph5: 15/45 | H.19: Handwriting beautify
- Ph5: 16/45 | H.20: Pattern brushes
- Ph5: 17/45 | H.21: Symmetry
- Ph5: 18/45 | H.23: Searchable handwriting index
- Ph5: 19/45 | H.24: Convert handwriting to text
- Ph5: 20/45 | N.11: Planners and calendars
- Ph5: 21/45 | N.12: Template packs
- Ph5: 22/45 | N.13: Version history per notebook
- Ph5: 23/45 | L.2: Offline transcription
- Ph5: 24/45 | S.13: Summaries from your highlights
- Ph5: 25/45 | S.14: Assignments with due dates
- Ph5: 26/45 | S.15: Language learning
- Ph5: 27/45 | S.16: Translate selection
- Ph5: 28/45 | F.2: Search a folder or the library
- Ph5: 29/45 | F.3: Full-text index
- Ph5: 30/45 | F.4: Search operators
- Ph5: 31/45 | F.5: Saved searches and smart folders
- Ph5: 32/45 | F.6: Library: collections, tags, read status
- Ph5: 33/45 | F.7: Metadata extraction
- Ph5: 34/45 | F.8: Duplicate detection
- Ph5: 35/45 | F.9: Folder watching
- Ph5: 36/45 | F.10: Citations
- Ph5: 37/45 | F.11: Zotero integration
- Ph5: 38/45 | F.12: Notes with backlinks and `[[links]]`
- Ph5: 39/45 | F.13: Export to Markdown and Obsidian with deep links
- Ph5: 40/45 | O.4: OCR to searchable PDF
- Ph5: 41/45 | E.1: Command-line batch tools
- Ph5: 42/45 | E.2: Agent tool table and `excellent-cli run`
- Ph5: 43/45 | E.3: MCP server
- Ph5: 44/45 | E.4: UI control channel
- Ph5: 45/45 | U.6: Neural offline voices

### Phase 6: 0 shipped; 0 in progress; 13 not started

**Not started (13)**

- Ph6: 1/13 | R.31: Visual search (Google Lens)
- Ph6: 2/13 | H.25: Math to LaTeX
- Ph6: 3/13 | S.9: Quizzes from a chapter
- Ph6: 4/13 | I.1: Providers
- Ph6: 5/13 | I.2: Document index for AI
- Ph6: 6/13 | I.3: Ask with cited answers
- Ph6: 7/13 | I.4: Summaries; I.5: Plain-language rewrite and explanation; I.7: Define terms in context
- Ph6: 8/13 | I.6: Explain equations step by step
- Ph6: 9/13 | I.8: Glossary, study guide and flashcard generation
- Ph6: 10/13 | I.9: Claim, method, finding and limitation extraction
- Ph6: 11/13 | I.10: Cross-document comparison and evidence tables
- Ph6: 12/13 | I.11: Suggested highlights and tags
- Ph6: 13/13 | I.12: Evaluation and safety suite

### Phase 7: 0 shipped; 0 in progress; 32 not started

**Not started (32)**

- Ph7: 1/32 | R.18: Thumbnail drag reordering
- Ph7: 2/32 | R.26: CAD drawing view
- Ph7: 3/32 | B.12: Other outline actions
- Ph7: 4/32 | A.23: Measurement
- Ph7: 5/32 | L.4: Screen and pen recording to video
- Ph7: 6/32 | G.9: Grading stamps
- Ph7: 7/32 | G.10: Rubric panel and scores
- Ph7: 8/32 | G.11: Comment bank
- Ph7: 9/32 | G.12: Batch open and export
- Ph7: 10/32 | G.13: Worksheet maker
- Ph7: 11/32 | F.15: PDF compare
- Ph7: 12/32 | F.16: Table and figure extraction
- Ph7: 13/32 | O.1: Page organizer
- Ph7: 14/32 | O.2: Split and merge
- Ph7: 15/32 | O.3: Crop pages
- Ph7: 16/32 | O.5: Edit text runs
- Ph7: 17/32 | O.6: Replace images
- Ph7: 18/32 | O.7: True redaction
- Ph7: 19/32 | O.8: Compress
- Ph7: 20/32 | O.9: Images to PDF and PDF to images
- Ph7: 21/32 | O.10: PDF to Markdown and DOCX
- Ph7: 22/32 | O.11: Forms: fill, validate and calculate
- Ph7: 23/32 | O.12: Save and reuse form values
- Ph7: 24/32 | O.13: Flatten forms
- Ph7: 25/32 | O.14: Sign documents
- Ph7: 26/32 | O.15: Validate signatures and trust
- Ph7: 27/32 | O.16: Signature-safe editing
- Ph7: 28/32 | O.17: Bates numbering
- Ph7: 29/32 | O.18: Headers, footers and watermarks
- Ph7: 30/32 | O.19: Permissions and encryption
- Ph7: 31/32 | O.20: PAdES long-term validation
- Ph7: 32/32 | O.24: Color-accurate and print-production view

### Phase 8: 0 shipped; 0 in progress; 3 not started

**Not started (3)**

- Ph8: 1/3 | K.12: Page canvas on Android
- Ph8: 2/3 | V.14: Two panes on tablets
- Ph8: 3/3 | O.21: Camera scan on tablets

### Phase 9: 0 shipped; 0 in progress; 13 not started

**Not started (13)**

- Ph9: 1/13 | W.13: Sync workspaces
- Ph9: 2/13 | G.14: Live classroom
- Ph9: 3/13 | G.15: Quick polls
- Ph9: 4/13 | F.14: Graph view
- Ph9: 5/13 | X.1: Excerpts on an infinite canvas
- Ph9: 6/13 | X.2: Connect, group and label
- Ph9: 7/13 | X.3: Evidence chains across documents
- Ph9: 8/13 | X.4: Export a board
- Ph9: 9/13 | Y.1: Folder sync with CRDT merge
- Ph9: 10/13 | Y.2: Same settings on every OS
- Ph9: 11/13 | Y.3: Share bundles
- Ph9: 12/13 | Y.4: Live co-editing through a relay
- Ph9: 13/13 | E.5: Plugins
