# Changelog

All notable work on ExcellentPDF is recorded here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), with one dated entry per piece of work. Times are local
(UTC−05:00) and mark when the work was finished.

## [Unreleased]

### 2026-11-14 16:11: Tabs (R.12)

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

### 2026-11-14 15:07: Reading themes, previewed live on the page (R.15)

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

### 2026-11-14 13:57: Smart dark mode: pages in the theme's colours, pictures kept (R.14)

#### Added

- **Page colours** (Settings, Reading): Original, Smart or Everything. Smart draws the pages in the theme's text and
  paper colours and leaves photos and artwork as they are; Everything recolours the pictures too. In a dark theme,
  links take the theme's accent colour.
- "Page colours: follow the theme" in the command palette, and Shift+I on the pages, switch between Original and
  Smart. The choice is kept.

#### Changed

- Selected text and the reading ruler's band show on dark pages: they are laid over them translucent.

### 2026-11-14 13:10: Settings window, and settings.toml followed as you edit it (K.16)

#### Added

- **Settings** (Ctrl+, or the command palette): appearance, the reading ruler, focus mode, auto-scroll speed, the
  links that open without asking, and every shortcut with its keys. Each change is saved as you make it.
- The Shortcuts section lists what in your `[shortcuts]` could not be bound, and opens `settings.toml` for editing.

#### Changed

- An edit of `settings.toml` takes effect within a second, keys included; no restart. A file broken by an edit is
  ignored (the old settings stay) until it is fixed.

### 2026-11-14 11:29: Page keys, your own shortcuts and commands that need a document (K.16)

#### Added

- Enhanced's plain keys work on the pages: `c` continuous, `[` `]` rotate, `g` go to page, `f` full screen, `j` `k`
  a line, Home and End, Backspace and Shift+Backspace back and forward, Enter and Shift+Enter a screen. They act only
  while the pages have the keyboard, so typing in a box is never hijacked.
- Your own shortcuts in `settings.toml`: `[shortcuts."go.to_page"]` with `keys = [...]` (anywhere) and
  `page_keys = [...]` (on the pages); `[]` removes a command's keys. Yours win over a default; keys that can't be
  bound and unknown command names are logged. Read at start, so a change needs a restart for now.

#### Changed

- The command palette leaves out page commands while no document is open.

### 2026-11-14 11:19: Full screen, focus mode, reading ruler and auto-scroll (R.19, R.20, R.21)

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

### 2026-11-14 08:51: Text selection and copy (K.13)

#### Added

- Text can be selected: drag across it, or double-, triple- or quadruple-click for a word, a line or a paragraph, and
  keep dragging to grow by the same unit, across pages if need be. The pointer is an I-beam over text.
- **Copy selected text** (`Ctrl+C`, `Ctrl+Insert`) and **Select all text on this page** (`Ctrl+A`).
- `excellent-text`, the text layer every text feature will read pages through.

#### Changed

- Page text is read without MuPDF's glyph-tight boxes, so a selection is an even band per line. Copied text joins
  words hyphenated across lines, as MuPDF's own text does.

### 2026-11-14 08:21: Outline (B.1, reading)

#### Added

- The sidebar's **Outline** tab shows the document's table of contents, with the page each entry opens. A press or
  the arrow keys go to an entry's page, and Back returns; the entry you are reading is marked and kept in view as you
  turn pages. A document without an outline says so.

### 2026-11-14 08:10: Thumbnails (R.5)

#### Added

- The sidebar's **Pages** tab shows every page small, with its label; the page you are reading is ringed, and a press
  goes to a page. Thumbnails render in the background below the pages on screen and are kept on disk, so a file opened
  again shows them at once.

#### Changed

- The render workers and the open documents are shared by the whole app (`excellent-core::shared`), not owned by the
  page canvas.

### 2026-11-14 07:49: Links (R.7)

#### Added

- Links work: a click on a cross-reference jumps there (Back returns), and the cursor shows a hand over any link while
  the title bar says where it goes.
- Web and mail links open in their app. A link to a file, a program or another protocol asks first; a script link
  never runs. The protocols that open without asking are the `link_protocols` setting (`http,https,mailto`).

### 2026-11-14 07:12: One parse per file, history, page labels and go to page (K.5, R.8, R.6)

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

### 2026-11-14 06:37: Page layouts, fit modes, reading direction, rotation and page boxes (K.6, R.1–R.4)

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

### 2026-11-14 00:38: Settings file; the theme is kept between runs (K.16)

#### Added

- `settings.toml` in the app's data folder (`%APPDATA%\com.excellentpdf.app`), or next to the exe in portable mode.
  Only changed values are written, unknown keys are kept, writes are atomic, and a file that doesn't parse is never
  overwritten.
- The theme and appearance you pick are kept and in place from the first frame of the next run.

#### Changed

- The theme picker takes the keyboard focus once it shows, and only then previews: the palette that opens it gave
  the focus back to what had it before, so arrow keys did nothing.

### 2026-11-14 00:32: Themes with live preview (K.17)

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

### 2026-11-13 17:03: render worker pool (T5, plan K.4); build-once rule

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

### 2026-11-13 16:42: safe MuPDF API (T4, plan K.3)

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
