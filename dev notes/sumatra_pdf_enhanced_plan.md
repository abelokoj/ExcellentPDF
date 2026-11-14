# SumatraPDF Enhanced: complete feature, settings and implementation reference

This document describes what SumatraPDF Enhanced does and how. Use it to rewrite the app in another language: every feature, every setting, every command, every command-line flag, the DDE interface, the policy file, the files the app writes, and how each subsystem works.

**Source snapshot.** `C:\dev\sumatrapdf`, branch `feature/study-reader-refinements`, commit `97c09d4ec` ("Date maintenance release 2026-11-09.8"). Enhanced is a fork of upstream SumatraPDF 3.7 (upstream commit `a0d8bcaed0412ce803d9c213845e710fcbe3c7a5`, 30 September 2026).

**How the lists were made.** The settings table (§6) and the command table (§7) were generated mechanically from the code generators, not copied by hand:

- settings: `cmd/gen-settings.ts` (`globalPrefsStruct`), plus 8 fields present only in the committed `src/Settings.h` (§6.3);
- commands: `cmd/gen-commands.ts` (`commandsRaw`, `commandAltDescs`) joined with the default keys in `src/Accelerators.cpp` (`gBuiltInAccelerators`);
- command-line flags: `cmd/gen-flags.ts`;
- DDE: `src/SearchAndDDE.cpp`.

Regenerate the tables from those files when the code changes. Feature descriptions come from the source, `docs/*.md` and `SumatraPDF_Enhanced_CHANGELOG.md`.

---

## 1. Product summary

A fast, single-executable Windows reader for PDF, XPS/OpenXPS, DjVu, ePub, MOBI/AZW, FB2, CHM, comic books (CBZ/CBR/CB7/CBT), images, PostScript, DVI, Markdown, HTML and plain text. Upstream SumatraPDF supplies the reader core. Enhanced adds:

- a reworked UI: rounded toolbar groups, twelve themes, bundled fonts, Lucide icons, a home page with a hero, cards and a learning hub;
- pen-first annotation: pen profiles, pressure, a lasso, a segment eraser, shapes, a laser pointer, pinned presets, Annotation Mode, recovery backups and transfer between PDF versions;
- object-level PDF dark mode and a CAD/engineering-drawing enhancement mode;
- reference hover previews for equations, citations and contents links;
- an offline dictionary, a vocabulary library and a spaced-repetition learning hub;
- read aloud with word highlighting;
- an AI chat sidebar that drives local agent CLIs (Claude Code, OpenAI Codex, Grok Build, Antigravity);
- PDF tools: merge, page editing, compress, encrypt, sign, bookmark editing through cpdf, and conversion;
- study export, screenshots, translation, Google Lens, a reading bar, auto-scroll, a page grid, tab groups, Explorer QuickLook and global hotkeys.

**Versioning.** The display version is `YYYY-MM-DD.N`, where N counts every published release and never resets. Windows file versions use `YYYY.MM.DD.N`. The updater accepts calendar versions and migrates from the older semantic versions.

---

## 2. Platform and build

| Item | Value |
|---|---|
| Language | C++ (MSVC, also clang and mingw), Win32 API; no STL containers. Own `Str`/`WStr`/`Vec`/`StrVec`/arena allocators live in `src/base/`. |
| PDF/XPS/EPUB engine | MuPDF (vendored at `ext/mupdf`, edited in place; every change is recorded in `ext/patches/*.patch` against `ext/versions.txt`) |
| Other engines | DjVuLibre (or own `EngineDjvuDec`), own CHM reader (`ChmFile`), own MOBI/PDB/FB2/LIT parsers, own DVI and PostScript engines (PostScript through Ghostscript if installed) |
| Image codecs | MuPDF, WIC, libwebp, AVIF and JPEG XL readers (`AvifReader`, `JxlReader`, `WebpReader`), and own TGA |
| Libraries (amalgamated `ext/a-*`) | brotli, bzip2, extract, freetype, gumbo, harfbuzz, jbig2dec, lcms2, libarchive, libwebp, mujs, openjpeg, unrar, zlib, zopfli |
| Web views | WebView2 (optional runtime) for Markdown, the HTML manual and the AI chat; `ext/marked.min.js`, `ext/mermaid.min.js`, markdown-it, cmark-gfm |
| Build | `bun cmd/build.ts -dbg/-rel/-asan/...` produces `out/<cfg>/SumatraPDF.exe` and `SumatraPDF-static.exe` |
| Binaries | `SumatraPDF.exe` (app plus installer), `libsumatrapdf.dll` (MuPDF and engines), `PdfFilter.dll` (Windows Search IFilter), `PdfPreview.dll` (Explorer preview and thumbnails), `sumatrapdf-tool.exe` |
| Embedded data | One LzSA archive (`IDR_EMBEDDED_PAK`): translations, JS libraries, the in-app manual and the installer payload |
| Architectures | x64, x86 and ARM64 |
| Code generation | `bun cmd/gen-code.ts` regenerates Settings.h/.cpp, Commands.h/.cpp, Flags.cpp, DocProp maps and the virtual-key tables |

---

## 3. Architecture

```
            ┌──────────────────────────── process ─────────────────────────────┐
 argv ──►   │ Flags.cpp ─► SumatraPDF.cpp (WinMain, message loop, single-instance│
 DDE  ──►   │              via DDE/WM_COPYDATA, ReuseInstance)                    │
            │                                                                     │
            │  MainWindow (one per top-level frame)                               │
            │   ├─ Toolbar / Tabs (TabsCtrl) / Menu / Caption / Sidebar           │
            │   │     sidebar: TableOfContents, Favorites, AIChatPanel            │
            │   ├─ Canvas (hwndCanvas: input, painting, selection, annotations)   │
            │   ├─ FindBar / Notifications / OverlayScrollbar / ReadAloud bar     │
            │   └─ WindowTab[] ──► DocController                                  │
            │                        ├─ DisplayModel (fixed-page: PDF, XPS, DjVu,  │
            │                        │      images, comics, PS, DVI, ebooks)      │
            │                        ├─ ChmModel (CHM in a browser control)       │
            │                        └─ MarkdownModel / BrowserDocView (WebView2) │
            │                               │                                     │
            │                        EngineBase ◄── EngineCreate (sniff + pick)    │
            │                         EngineMupdf, EngineDjvuDec, EngineEbook,     │
            │                         EngineImages, EnginePs, EngineDvi            │
            │                               │                                     │
            │  RenderCache + PageRenderService (worker threads, tile bitmaps)     │
            │  Settings (gSettings, GlobalPrefs) ◄─► SumatraPDFEnhanced-settings.txt│
            │  Commands (CmdId) ◄── Accelerators / Menu / Toolbar / CommandPalette │
            └─────────────────────────────────────────────────────────────────────┘
```

**Key types**

- `MainWindow`: a top-level frame. It owns its child HWNDs, the current tab, the annotation placement state (`annotPlacement`), the pen style, the selection and the find state.
- `WindowTab`: one open document. It holds the file path, its `DocController`, nav history, per-tab view state (reading bar, page color mode) and the annotation edit state.
- `DocController`: the abstract controller. Fixed-page documents use `DisplayModel`, which lays pages out by display mode and zoom, maps between screen and page coordinates (`DocumentLayout`, `PagePosition`), scrolls and hit-tests.
- `EngineBase`: page count, page size and rotation, `RenderPage(args)` → bitmap, text extraction with per-glyph coordinates, links, TOC, page labels, properties, annotations, save, and `HasClipOptimizations`.
- `RenderCache`: caches rendered tiles keyed by (doc, page, rotation, zoom, tile index). `PageRenderService` and `PageRenderPolicy` schedule renders on background threads, cancel stale requests, and prefetch nearby pages.
- Settings: `GlobalPrefs`, generated as a struct tree with metadata tables. The parser reads and writes a sparse text format (§6.1) and only writes values that differ from the default.
- Commands: `CmdId` integers are assigned by position in `gen-commands.ts`. Each command has a name, a description, optional alternative names (palette search) and optional arguments. Shortcuts can call commands with arguments (`Shortcuts[].Cmd` = `CmdId arg`).
- `CommandAvailability`: one place decides whether a command is enabled for the current tab, engine and selection. The menu, toolbar, selection toolbar and palette all ask it.

**Threading rules.** UI work stays on the UI thread. Rendering, text extraction, thumbnails, dictionary downloads, AI process I/O, recovery snapshots, cpdf and update checks run on worker threads. Their results come back through `uitask::Post`. Every result checks that its tab still exists and still holds the same document before it is applied. The temp arena is reset on every message-loop iteration, so never keep `*Temp` strings.

---

## 4. Supported file formats

| Engine | Kind | Formats |
|---|---|---|
| EngineMupdf | `kindEngineMupdf` | PDF (including encrypted, damaged and repaired files), XPS/OpenXPS, EPUB (fixed layout through MuPDF), FB2 through MuPDF, CBZ/SVG/image formats MuPDF reads |
| EngineDjvuDec | `kindEngineDjVu` | DjVu |
| EngineEbook | `kindEngineEpub`, `kindEngineMobi`, `kindEngineFb`, `kindEnginePdb`, `kindEngineChm`, `kindEngineHtml`, `kindEngineTxt` | ePub, MOBI/AZW/AZW3, FB2/FB2Z, Palm PDB/eReader, CHM (as an ebook), HTML, TXT (laid out by own HtmlFormatter / EbookFormatter) |
| EngineImages | `kindEngineImage`, `kindEngineImageDir`, `kindEngineComicBooks` | PNG, JPEG, GIF (animated: first frame), BMP, TIFF (multipage), TGA, WebP, AVIF, JXL, HEIC (when WIC has a codec), JPEG 2000; a directory of images; CBZ/CBR/CB7/CBT (zip, rar, 7z, tar through libarchive/unrar) |
| EnginePs | `kindEnginePostScript` | PS and EPS, converted to PDF through an installed Ghostscript |
| EngineDvi | `kindEngineDvi` | DVI |
| ChmModel | (controller) | CHM in the HTML help viewer |
| MarkdownModel | (controller) | `.md`/`.markdown` rendered in WebView2, with a TOC of headings and folder navigation (`MarkdownToc`) |

`EngineCreate` sniffs the file content first and falls back to the extension. Archives are classified by their entries. The `-html-backend` flag selects the HTML/EPUB backend.

---

## 5. Feature catalogue with implementation notes

Each heading gives what the user sees, then how it works. Command IDs (`Cmd…`) are listed with their shortcuts in §7, and setting names are listed in §6.

### 5.1 Opening, tabs, windows, sessions

- **Open:** file dialog, drag and drop, command line, DDE `[Open]`, recent files, `CmdOpenNextFileInFolder`/`Prev`, and reopening the last closed tab.
  - With `ReuseInstance`, a second launch forwards its arguments to the running instance.
  - Passwords are tried from `DefaultPasswords` and remembered per file, then the user is prompted (`GetPasswordDialog`).
- **Tabs:**
  - Behaviour: MRU Ctrl+Tab (`CtrlTabSimple` switches to plain order); close others/close to the right; drag a tab out to a new window; middle-click to close.
  - Appearance: the page number in the tab (`ShowPageNumberInTabs`), `TabWidth`/`MinTabWidth`, and the active-tab style and accent (`ActiveTabStyle`, `ActiveTabAccent`).
  - Overflow: scroll buttons `<` `>` and a `v` list. The list (the open-document dropdown) has live, case-insensitive filename filtering, keyboard selection and a "no matches" state; `TabListVisibleItems` sets its height.
- **Windows:**
  - Multiple top-level windows; `CmdNewWindow`, and `-new-window`/`-new-window-tabs`.
  - The window position and state are saved (`WindowPos`, `WindowState`).
  - The menu bar can be shown or hidden (`ShowMenubar`, `ShowMenubarWithTabs`).
- **Sessions:** `RestoreSession` reopens the tabs of every window (`SessionData[]`, with the active tab, the per-tab view state and the window geometry). `ReopenOnce` reopens once after an update or restart.
- **Tab groups** (`TabGroupsManage`):
  - Save the current set of tabs as a named group (`TabGroups[]`: name and `TabFiles[]` with paths and states).
  - The manage dialog restores, renames and deletes groups. A restored group reuses the current window when that window has no files open.
- **File history and Frequently Read:**
  - `FileStates[]` stores per-file state:
    - path, open count, last opened, favorites, page, scroll, zoom, rotation, display mode;
    - TOC expansion state, password, and the decryption key (when it is remembered);
    - per-document overrides.
  - `RememberStatePerDocument` and `RememberOpenedFiles` control saving. `OpenCountWeek` ages the counts weekly.
- **Home page (about tab):**
  - Contents: a hero with search, recent-document cards with thumbnails, Resume last, a learning-hub card, and day/night/inversion actions.
  - Layout: `HomePageViewMode` (`thumbnails` or `list`), and thumbnail size with +/− and Ctrl+wheel.
  - Options: full-page thumbnails, a fixed header, sort by Frequently Read, maximum items, and a reading-progress display.
  - Thumbnails (`FileThumbnails`) render on bounded background workers and are stored as PNG in the app data folder. Undersized thumbnails are re-rendered at a higher resolution while the old one stays visible.
- **Favorites:**
  - Each favorite stores a page, a name and a position on the page.
  - The sidebar shows a favorites tree and a menu with optional sort by name. Lookups use full paths, so two files with the same name in different folders stay separate.
- **Navigate files in folder** (`NavFilesInFolder`):
  - A modeless directory browser that lists the files SumatraPDF can open.
  - Enter opens a file in the current tab; Ctrl+Enter opens it in a new tab or switches to the tab that already shows it.
  - Back/Forward/Up/Home; Home lists drives and Explorer's quick-access locations.
- **Reload:** `ReloadModifiedDocuments` watches the file (a directory change notification) and reloads it, keeping the page and zoom.
- **Explorer QuickLook** (`ExplorerQuickLook`, default off):
  - In File Explorer or on the desktop, Space previews the selected file in a popup window. Esc or Space closes it; Left/Right open the previous or next file.
  - A background agent process (`-quicklook-agent`) runs a top-level window. Starting it again finds and stops older agents. The agent never blocks log-off.
  - `-quicklook <file>` opens the preview window.
- **Global hotkeys** (`GlobalHotkeys`): system-wide bindings for chosen commands, made with `RegisterHotKey` and handled on `WM_HOTKEY`.

### 5.2 Viewing and layout

- **Display modes:** single page, facing, book view (cover alone), continuous, continuous facing, continuous book, and page aspect. Page aspect means:
  - a portrait first page opens as continuous + fit width;
  - a landscape first page opens as single + fit page;
  - a remembered state still wins.
- **Zoom:**
  - Modes: fit page/width/height/content/visible, and custom zoom with an editable toolbar field.
  - Steps: `ZoomLevels` lists the zoom-in/out stops. Above 100%, `ZoomIncrement` adds 25-point steps.
  - Other: Ctrl+wheel and pinch zoom; `CustomZoomDialog`; `IgnoreDestinationZoom` ignores zoom values in links.
- **Rotation:** in 90° steps, per document.
- **Presentation and fullscreen:**
  - Presentation mode uses a black or white background, hides the cursor and turns pages on click.
  - Fullscreen hides the chrome. `PreventSleepInFullscreen` keeps the screen on, and `EscToExit` lets Escape exit.
- **Manga mode:** right-to-left facing pages for comics (`ComicBookUI.CBXMangaMode`).
- **Scrolling:**
  - `SmoothScroll` and `ScrollLineAmount`.
  - Page turning: `MouseWheelTurnsPage` for single page, `ScrollEdgeTurnsPage`, `ClickEdgeToTurnPage`, and `RememberViewOffsetOnPageTurn`.
  - `FastScrollOverScrollbar` scrolls quickly when the wheel is used over the scrollbar.
- **Overlay scrollbars** (`OverlayScrollbar`):
  - `Scrollbars` = `windows` (standard), `smart` (overlay that hides itself), `overlay` (always visible) or `hidden`.
  - The overlay scrollbars are custom-drawn in theme colours. A global list of live scrollbars tracks the mouse.
  - `ScrollbarWidth` and `ScrollbarWidthExpanded` set the width; `ScrollbarInSinglePage` controls single-page mode.
- **Reading bar** (`ReadingBar`):
  - A horizontal band in the viewport. Highlight mode fills it; Invert mode dims everything else.
  - Each tab stores whether it is on and its vertical position (yFrac). Colour, invert and height are global (`ReadingBar.*`).
- **Auto-scroll** (`ReadingAutoScroll`):
  - Hands-free, continuous panning at a stored speed (`ReadingAutoScrollSpeed`), with finer steps than Acrobat's 0–9.
  - The arrow key that matches the pan direction speeds it up; the opposite arrow slows it. Middle-click auto-scroll is a separate mode.
- **Page grid** (`PageGridDialog`): a measurement grid over pages with spacing, origin, colour and line style, saved in `FixedPageUI.PageGrid`. Show Grid is a session toggle.
- **Links:**
  - `ShowLinks` outlines links; `HighlightLinkDestination` flashes the target; `DisableLinks` and `DisableAutoLinks` turn links off.
  - Links to other files: protocols allowed by restrict.ini are opened by the OS. File types listed in `SafeFileTypes` open externally.
- **Document focus indicator** (`ShowDocumentFocusIndicator`) and the document background (`MainWindowBackground`, `FixedPageUI.BackgroundColor`/`TextColor`/`GradientColors`).

### 5.3 Rendering, colours and dark mode

- **Tiled rendering:**
  - Each page splits into tiles sized from the zoom. Tiles render at the target resolution, with a lower-resolution tile as a placeholder while it renders.
  - Memory use is bounded; `SaveMemory` reduces it further.
  - `DisableAntiAlias` turns off anti-aliasing. `CustomScreenDPI` overrides the screen DPI.
- **Invert colors / document colours:**
  - `FixedPageUI.InvertColors` and the `-invert-colors` flag invert the document.
  - Invert presets: `InvertColorIndex` selects one of presets 0–5 or custom presets 6–7, which use `InvertCustomLightCol`/`InvertCustomDarkCol`.
  - `DocumentColorsFollowTheme` maps the page text and background colours to the theme. Its values are Off, Smart and Legacy.
- **Object-level PDF dark mode** (`PdfDarkMode*.cpp`, about 3,500 lines):
  - **Renderers** (`PdfDarkModeRenderer`): `LegacyBitmapPostProcess` (inverts the rendered pixels) and `ObjectLevelDevice` (a custom `fz_device` that recolours each draw call).
  - **Per-page modes** (`PageColorMode`): `Normal`, `LegacyInvert`, `SmartDark`, `PreserveImages`, `ScanDark`.
  - **Colour mapping:** text and vector fills and strokes are recoloured in OKLab. Lightness is inverted around the theme's background and text colours; chroma and hue are kept, so coloured highlights stay recognisable.
  - **Image classification** (`ImageClassifier`, `ImageStats`, `ImageRules`) separates photos (kept) from line art and scans (darkened). Page-1 covers are kept.
  - **Scan detection** (`DarkModeOptions`): a page counts as a scan when one image covers at least 75% of it, the dominant image covers at least 85%, the aspect skew is at most 1.15, and the page has at most 10 text operations and at most 20 vector operations. Such pages get `ScanDark` processing.
  - **Other:** near-white pixels of preserved images can be blended toward the page background. Results are cached per page and per engine (`PdfDarkModeCache`, `EngineCache`). PdfFilter and PdfPreview link a no-op version.
  - **Settings:** `DocumentColorsFollowTheme` and the per-document colour mode commands.
- **CAD / engineering-drawing enhancement** (`PdfCad`):
  - `EngineeringDrawingEnhance` = off/auto/on. The manual toggle wins over the global mode, which wins over auto-detection.
  - **Detection (auto):** an analysis device scores each page from statistics:
    - stroke count and the ratio of thin strokes (≤ 0.25 pt);
    - the stroke-to-fill ratio;
    - the largest image's coverage, and the number of text operations.
  - **Two positive patterns:**
    - "hairline CAD": image coverage < 5%, at least 40 strokes, thin ratio > 0.25 and stroke/fill > 0.55;
    - "raster CAD": one image covers at least 80% of the page, with fewer than 50 vector operations and fewer than 200 text operations.
  - **Exclusions:** book-like documents (many pages with a large image and a lot of text) are excluded.
  - **Rendering:** a wrapping `fz_device` passes stroke, fill and text calls through `CadMapColor`, which maps colours to high-contrast RGB so faint and coloured lines stay readable.
- **Themes** (`Theme.cpp`, `Themes[]`):
  - Built-in themes: Light, Dark, Darker and the twelve Enhanced presets (Pure White, Warm Sepia, Soft Paper, Pure Black, Midnight Charcoal, Warm Espresso and others), plus System (follows `AppsUseLightTheme`).
  - Each theme sets colours for text, background, controls, active and inactive tabs, links, disabled text and muted text, plus `Colorize…` options.
  - Users can add themes in `Themes[]`; renamed themes are migrated. `ToggleLightDarkTheme` switches between `LastLightTheme` and `LastDarkTheme`.
  - `ChangeThemeDialog` and `ChangeColorDialog` edit themes with live preview (`LiveAppearancePreview`); Cancel restores the saved values.
- **UI fonts and scale:**
  - Fonts: Manrope, Pretendard and Public Sans (Regular and SemiBold, SIL OFL) are embedded as private resources and loaded with `AddFontMemResourceEx`. They are selected with `UIFontFamily` or System.
  - Sizes: `UIFontSize`, `TreeFontName`/`TreeFontSize`, and `InterfaceScale`.
  - Icons: Lucide SVG icons (`SvgIcons.cpp`) are rendered to theme-coloured bitmaps and re-rendered when the theme changes. Icon styles: `AnnotationModeIconStyle`, `ColorPickerIconStyle`, `PinIconStyle` and `FileTypeIconStyle`.

### 5.4 Navigation

- **TOC sidebar** (`TableOfContents`):
  - Shows the PDF outline and EPUB/CHM navigation. `AutoGenerateTOC` builds one from headings when the document has none. Page numbers are optional (`ShowTocPageNumbers`), and the expansion state is saved per file.
  - The TOC is filterable. Right-click offers copy, export and Edit PDF bookmarks.
  - Sidebar layout: `SidebarDx`, `TocDy`, `SidebarOnRight`, `AlwaysShowSidebar` and `SidebarBottomView`.
- **In-place bookmark editing** (`CpdfBookmarks.cpp`): for a PDF on disk, the TOC context menu offers Add (current page), Rename and Delete. Changes are saved on a background job through the verified cpdf executable (§5.10). Without editing rights, Ctrl+B adds a favorite.
- **Chapters in ebooks** (`ShowChaptersInEbooks`, `ChapterTable`): shows the current chapter as a location.
- **Go to page** (`GoToPageDialog`): accepts page labels, including Roman numerals. Also first/last/next/previous page, back/forward nav points, and named destinations (`-named-dest`).
- **Location history:** each jump adds a nav point. Back and forward use Alt+Left/Right.

### 5.5 Search

- **Find bar** (`FindBar`), docked or floating (`SearchUIFloating`, `SearchUIWindowPos`):
  - Options: match case and whole word. Ranges: forward/back, F3/Shift+F3.
  - Every match on visible pages is highlighted.
- **Find window** (`FindWindow`): a list of all matches with snippets. `FilterHighlightDraw` highlights the matched words; with whole word on, matches inside longer words are skipped.
- **TextSearch:**
  - Searches each page's extracted text and glyph coordinates. A match across a line break counts when the break is whitespace or a hyphen.
  - Search runs on a background thread and can be cancelled. The UI shows progress.
- **Annotation search and filter** (`AnnotSearch`, `AnnotFilterToolbar`):
  - The filter box takes plain words plus `:conditions` such as `:type=highlight`, `:author!=x` or `:color=…`. Conditions are ANDed; the operators are `=`, `==` and `!=`.
  - A typo in a condition makes the box fall back to plain-text matching, not to "no results".
  - The list highlights matched words. Selecting an entry jumps to the annotation.
- **Search selection with**: `SelectionHandlers[]` (name, URL with `${selection}`, key, and icon) send the selected text to a web search, dictionary or translator.

### 5.6 Selection, copy, translate, capture

- **Text selection:** mouse drag, double-click selects a word, triple-click a line. Keyboard selection (`SelectTextKeyboard`) uses caret navigation with Shift. Ctrl+drag makes a rectangle (area) selection.
  - Copy produces text, plus an image when the selection is a rectangle.
  - `CopySelection` in restrict.ini can block copying.
- **Selection highlight presets:** editable colours for the selection overlay on fixed-page documents (`FixedPageUI.SelectionColor` and presets).
- **Selection toolbar** (`SelectionToolbar`, ported from sumatrapdf-plus):
  - A floating bar appears under or over a finished selection, with copy, highlight, read aloud, translate, look up, search-with and other buttons.
  - `SelectionToolbar` turns it on, and `SelectionToolbarLayout` sets the layout.
  - Each button checks `CommandAvailability`.
- **Translate selection** (`SelectionTranslate`):
  - A dialog with editable From/To language dropdowns that map names to ISO 639 codes. An unknown source language means auto-detect; an unknown target language means English.
  - The engine (`TranslateEngine`) is Google or DeepL, opened at `translate.google.com` or `www.deepl.com/translator` with the text in the URL, or one of the AI agents (Grok Build, Claude Code, OpenAI Codex, Antigravity). `TranslateFromLang` and `TranslateToLang` are remembered.
- **Google Lens:**
  - The current page is rendered to PNG (at most 32 MB) and written into a temporary local HTML page.
  - That page holds a multipart form that posts itself automatically to `https://lens.google.com/v3/upload`. It opens in the default browser.
- **Screenshots** (`ScreenshotCapture`, `Screenshot`):
  - A picker overlay shows a grid of thumbnails of screens and windows. It is shared with other apps through `ScreenshotHost` hooks.
  - You can also capture a region of the document.
- **Save, crop and resize images** (`ImageSaveCropResize`):
  - Save a selection or page as PNG/JPEG/WebP, with crop and resize.
  - `None` interpolation is used at 1:1 and integer scales, so the image keeps its pixel grid.
- **PNG optimiser** (`PngOptimizer`):
  - Recompresses PNG output with zopfli. Large files are skipped. Images above a pixel limit use one filter and one iteration instead of every filter at 15 iterations.
  - Adds a `tEXt` "Software" chunk.
- **Text snippets:** `TextSnippets[]` stores named text snippets to insert into free-text annotations.

### 5.7 Reference hover previews (`RefHover*.cpp`)

**What it does:** hovering over an internal link (a citation, equation, figure or contents entry) shows a resizable popup with that region of the destination page rendered.

- **Detection:**
  - Link destinations (page, x, y) come from the PDF link annotation.
  - When there is no link, numeric citations ("[12]", "(3)") and author-year citations are matched against the references section (plain-text lookup in `RefHoverText`).
  - Black and coloured link text use the same path. When the first mouse hit misses, the cursor position alone finds the link.
- **Region detection** (`RefHoverDetect.cpp`, pure functions tested with synthetic glyph arrays):
  - `NormalizeGlyphLines` groups glyphs into lines. `StripWatermarkGlyphs` removes rotated watermark text.
  - `DetectEntryBox` finds the reference entry under the destination point, using hanging indents and the next entry start. When the entry continues in the next column, `FindColumnWrapContinuation` follows it.
  - `DetectEquationBox` uses the equation number on the right edge and the vertical gaps.
  - `LandscapeBox` handles rotated pages. Caption labels ("Figure", "Table") get special treatment.
  - When the destination y is near the bottom of the page (`ShouldSearchNextPage`), the next page is searched too.
  - Every box is clipped to the mediabox and padded.
- **Popup:**
  - Rendering is asynchronous (`RefHoverRender`). The popup has a 0.6 pt red outline scaled for DPI; in high-contrast mode it uses the system edge colour.
  - Its initial width is at most 95% of the source page width, including the frame. All four edges can be dragged to resize it (`RefHoverEdgeHit`).
  - Text in the popup can be selected and highlighted. Highlights become real PDF highlights with normal undo and save, and the main document's selection is kept.
- **Lifecycle:**
  - The popup appears after `CitationHoverDelay` ms.
  - Moving inside the same reference keeps the deadline and the render request. Leaving or changing the reference cancels or replaces them.
  - Known references survive brief hit-test misses while a page renders.
  - It works in the reading and hand tools while idle, but not while dragging.
  - The one-line tooltip for internal links is suppressed when a preview is available.

### 5.8 Annotations

**Types.** Text (sticky note), free text (with font, size and colour), highlight, underline, strikeout, squiggly, ink, line (with arrow and dimension variants), rectangle, ellipse, polygon (N sides, `PolygonSides`), star (N points, `StarSides`), polyline, math curves and axes, stamp, file attachment, caret, redact (where MuPDF supports it), and link.

**Placement** (`AnnotPlacement.cpp`):

- A placement records points in page coordinates, so saved geometry does not depend on zoom. One annotation covers one page.
- While you draw, a GDI+ overlay previews the pending annotation. Finishing calls `AnnotationPlacementFillCreate`, which passes the page, points, stroke counts, pen profile, colour, opacity and border width to `Annotation.cpp`. That code creates the MuPDF annotation and its appearance.

**Pointer input** (`PointerInput.h`):

- Windows 8 pointer APIs are loaded dynamically. Each `PointerSample` records:
  - device type (mouse, touch or pen) and screen coordinates;
  - millisecond and QPC timestamps;
  - contact and button state, barrel, inverted and eraser flags;
  - pressure (0–1, only when the driver's validity mask says it is valid), and tilt and rotation with their own validity flags.
- Up to 128 coalesced history samples are read oldest first. Pen-eraser contact runs the eraser. While writing, touch input is suppressed.

**Ink and pens:**

- Profiles: ballpoint, fountain, brush, pencil and highlighter.
- Thickness: `PenMinWidth`/`PenMaxWidth`/`PenWidthStep`, default 0.1–16 pt in 0.1 pt steps. Fractional widths survive saving.
- Pressure: fountain and brush strokes get one width per stroke from the mean pressure, because PDF ink has one width per stroke:
  - fountain: `w × (0.3 + p)`;
  - brush: `w × (0.25 + 1.5 p)`;
  - clamped to the minimum and maximum width.
- Smoothing: the live stroke is drawn as a cardinal spline (GDI+ `DrawCurve`, tension 0.5, round caps), and the window repaints immediately.
- Profile metadata: the pen style is written into the ink annotation dictionary, so reopened strokes can be edited and erased the same way, on any Enhanced install.
- Pen cursor: `PenCursorStyle`, `PenCursorSize` and `PenCursorAccent`.

**Laser pointer:**

- Styles: solid trail, hollow trail and single dot, in preset or custom colours (`LaserColors`).
- Width: `LaserWidth`, in DPI-scaled screen pixels.
- Lifetime: `LaserLifetimeSeconds` after lift.
- Laser marks are separate canvas state and are never saved.

**Shapes:** arrowheads are filled and sized in proportion, both in MuPDF appearance streams (patched `pdf-appearance.c`) and in the app's preview. Polygon and star vertices are generated in `AnnotPlacement`.

**Erasers:**

- Stroke eraser: `InkStrokeHit` tests segment distance and removes whole strokes.
- Segment eraser: clips a circle out of InkList polylines and keeps the pieces. Shapes, including rectangles, ellipses, polygons, stars and curves, can also be erased segment by segment.
- Highlight-only eraser.
- One eraser gesture is one undo step.

**Lasso** (`Canvas.cpp`):

- Draw a closed path to select fully enclosed annotations on one page. An ink annotation is selected only when all its points are inside.
- Handles: move, resize and rotate. The rotate handle is a circular badge, and Shift snaps rotation to 15°.
- Also: arrow keys, Delete, duplicate, recolour and change thickness.
- Each action is one MuPDF journal operation. A live preview is drawn while transforming. The selection clears when the tab or tool changes.
- `LassoRectangular` switches to a rectangle lasso.

**Edit toolbar** (`AnnotEditToolbar`): edits the selected annotation's colour, opacity, border width, font, contents, author, icon and line endings. `ShowAnnotationAuthorInTooltip` and `ShowAnnotationNotification` control related display.

**Pinned tools:**

- `PinnedAnnotationTools[]` stores tool, colour, width, opacity and pen style. The same pen can be pinned several times with different settings.
- Pinned tools appear on the toolbar in the order added. Right-click removes one.
- `ToolbarAnnotationOrder` sets the order of annotation tools.

**Annotation Mode:**

- Hides the title bar and tabs; Escape exits.
- The toolbar can be docked to any edge.
- Side layout: 1–3 columns (`AnnotationToolbarColumns`) in Grouped or Continuous rail style (`AnnotationToolbarStyle`).
- Page-number layout: `AnnotationPageNumberStyle` (1A, 1B, 1C) and `AnnotationThreeColumnPageStyle`.
- Tools that do not fit can be reached by scrolling.

**Highlights:** select text and press A to highlight it. To highlight an area, Ctrl+drag and then press A. New highlights have straight sides and rounded corners.

**Undo and redo:** use MuPDF's journal. `BeginPdfEditOperation` groups a gesture into one step. The undo history lasts only for the session.

**Saving:** `EngineMupdfSaveUpdated` saves incrementally when possible and fully otherwise. Save As is also available.

**Recovery** (`AnnotRecovery.cpp`):

- Unsaved annotation changes are debounced into atomic PDF snapshots plus a versioned manifest in a separate recovery folder. The manifest stores the source size, modification time and SHA-256.
- An exclusive session lock stops another instance from offering a backup that is in use.
- On reopen, the app offers to open the recovered copy, discard it, or keep it.
- Saving or discarding cancels recovery.
- Recovery needs a PDF that can be saved incrementally.

**Transfer to a revised PDF** (`AnnotationTransfer.cpp`, `…Dialog.cpp`):

- Export writes a package `.sannot.pdf` that holds the annotations, matching data, the original path and the source text.
- Import shows a mapping from old pages to new pages:
  - pages are matched by unique text anchors, so they follow inserted or reordered pages;
  - the user can set a page manually, or enter 0 to skip it.
- The result is saved as a new PDF. Password protection is kept, and importing the same package twice does not duplicate annotations.
- Active content is stripped, including content hidden inside appearance resources.

**Free-text fonts:** use PDF resources, and embed fonts where the license allows.

### 5.9 Forms and JavaScript (`FormFields.cpp`, `docs/pdf-form-filling-plan.md`)

- **Fields:** fill text, checkbox, radio, combo box and list box fields in place.
  - Only one field is edited at a time: a floating Win32 edit box or list box over the page.
  - Commit writes the value through `pdf_set_field_value` and similar calls. Escape cancels.
- **Highlighting:** empty fillable fields get a pale blue tint (`HighlightFormFields`).
- **JavaScript:** form calculations run through MuJS (`DisableJavaScript` turns them off).
- **External images:** off by default (`AllowExternalImages`).
- **Signatures:** signature fields can be signed (§5.10).

### 5.10 PDF tools

- **Page tools** (`PdfTools.cpp`): each dialog takes a source, a destination and options. They are:
  - extract pages, delete pages, page range;
  - compress, decompress;
  - encrypt, decrypt;
  - bake, which flattens annotations and forms;
  - extract text;
  - convert PDF to images, convert to PDF;
  - save selection as image.
  - All work runs in the background. Results are always saved as a new file.
- **Merge PDF** (`MergePdf.cpp`):
  - Shows the pages of the document and of added PDFs as a grid of thumbnails.
  - Drag pages to reorder them. Removing a page only marks it, so it can be restored.
  - Save over the document or as a new PDF.
- **Edit PDF bookmarks** (`CpdfBookmarks.cpp`, `docs/cpdf-bookmarks.md`):
  - Uses an external cpdf.exe. The download is verified: 11,724,417 bytes, SHA-256 `91f51d30…7351`, upstream commit `38b2556d`.
  - The editor works on a snapshot with writes denied while it copies.
  - You can edit title, page, level and expanded state, and add, remove, move up or move down.
  - Only the edited fields are replaced; styles and actions are kept. Changing the page creates a Fit-page destination.
  - The result is saved as a new file without replacing an existing one, using Windows argv quoting with no shell.
- **Sign** (`PdfSign.cpp`, `SignDocumentDialog.cpp`, `EutlTrust.cpp`):
  - Signing fills a signature field with a PKCS#7 signature (`pdf_sign_signature`). The certificate comes from a `.pfx`/`.p12` file (`PFXImportCertStore`, with a password) or from the Windows "MY" store (`CertOpenSystemStoreW`).
  - Trust is checked against the EU Trusted List: the LOTL (List of the Trusted Lists) is fetched, and the XML TSLs (trust-service status lists) it names are loaded.
  - Signing is app-only; the shell extensions do not include it.
- **Document properties** (`DocumentProperties`): metadata, fonts, page size, PDF version, encryption and permissions. Labels are right-aligned and values left-aligned, and long values wrap.

### 5.11 Printing (`Print.cpp`, `PrintWin11.cpp`)

- **Print dialog:** the system dialog plus a Windows 11 style dialog.
- **Options:**
  - Range: page ranges, odd/even pages, the current selection.
  - Layout: scaling (shrink, fit, none), rotation and centring.
  - Output: colour or grayscale, copies, duplex and paper source.
- **Remembered defaults:** `PrinterDefaults`, and the print UI choice `PrinterUI`.
- **Command line:** `-print-to`, `-print-to-default`, `-print-dialog` and `-print-settings` (tokens: page ranges, `odd`, `even`, `noscale`, `shrink`, `fit`, `color`, `monochrome`, `duplex*`, `bin=`, `paper=`, `Nx` copies, `stretch`, `center`).
- **Policy:** `PrinterAccess` in restrict.ini can block printing.

### 5.12 Dictionary, vocabulary and learning hub

**Offline dictionary** (`OfflineDictionary.cpp`):

- Sources, in priority order:
  1. the bundled wmkeyboard study packs: 11 packs and 3,890 unique meanings, in WM JSON;
  2. installed and imported dictionaries;
  3. WordNet English as a fallback for ordinary words, embedded or loaded from a folder.
- Import formats: TSV, StarDict (`.ifo`/`.idx`/`.dict[.dz]`, honouring `sametypesequence`), WM JSON, and gzip-compressed versions of these. Imports are validated: UTF-8 only and bounded sizes.
- Lookup:
  - `WordKey` normalises the word: it trims punctuation, turns `_` into spaces, and lowercases with the invariant locale.
  - Inflections (`DictInflection`) map inflected forms to their base form.
  - Meanings are merged with their part of speech.
- Shift+D (or the menu) looks up the selected word in a native, themed dialog that also accepts typed words.
- `DictionaryUI.DictionaryDesign` picks the dialog layout: Classic, Curated Lexicon or Deep-Work Inspector.
- Aggregation mode merges results from all downloaded sources and online sources. Each result is labelled with its source, lookup can be cancelled, and the source selection is saved.
- No selected text is sent online, except through the explicit online-dictionary and recording commands.

**Kaikki (Wiktionary) packs** (`KaikkiDictionary.cpp`):

- Download: language packs (`KaikkiPack`: id, URL, size) are downloaded over HTTPS only through WinINet. There are no redirects, no cookies, a 30 s timeout, and ETag/Last-Modified checks. Free disk space is checked first.
- Indexing: the JSONL records are indexed by headword and by `forms[].form`. Keys are FNV-1a 64-bit hashes stored in bucket files inside a "generation" folder, with u64 offsets, so packs can exceed 4 GiB.
  - A hash collision cannot return the wrong word, because each candidate's key is compared.
  - A new generation is built in full, then a small "current" pointer is switched to it.
- Pack files: `manifest.json`, `index.bin` and `records.jsonl`.

**Pronunciation** (`DictionarySpeech.cpp`):

- Words are spoken with text-to-speech.
- Recordings are fetched only by the explicit recording command, with bounded reads and no redirects. Cancelled workers free their own state.

**Vocabulary library** (`Vocabulary.cpp`, stored in `SumatraPDF-vocabulary.json`):

- Each word stores:
  - the definitions and dictionary ids;
  - the reading source: the file, page and context sentence;
  - learned status and deck membership;
  - scheduling: box, ease, repetitions, intervalDays, dueTime, lapses, reviews and lastReviewed.
- Saving is atomic. A corrupt store is protected: it becomes read-only instead of being overwritten.
- Removing a word can be undone.
- Other features: search, custom lists, backup/import/export with attribution, and the guide and layout files (`SumatraPDF-learning-guide.txt`, `SumatraPDF-learning-layout.txt`).

**Scheduling** (`VocabularyReview`, with grades Again, Hard, Good and Easy):

- **Leitner:**
  - The intervals for boxes 0–4 are 1, 3, 7, 14 and 30 days.
  - Again resets to box 0. Hard keeps the box. Good adds 1 and Easy adds 2, up to box 4.
  - A word counts as learned when it was already in box 4 and gets Good or Easy.
- **SM-2:**
  - The grades map to quality 1, 3, 4 and 5.
  - ease = clamp(ease + 0.1 − (5−q)(0.08 + 0.02(5−q)), 1.3, 10).
  - The interval is 1 day, then 6 days, then max(prev+1, round(prev × ease)). Easy multiplies it by 1.3 from the third repetition on.
  - Again resets repetitions to 0. A word counts as learned when its interval is at least 60 days.
- Again also counts a lapse and clears learned. The interval is clamped to [1, kMaxReviewDays].
- "Mark learned" sets box 4 with a 30-day interval.

**Learning hub** (`VocabularyDialog.cpp`, on the home page):

- Decks and reviews: 11 named study decks, word of the day, due reviews and study ahead.
- Six activities: flashcards, meaning choice, word choice, spelling, word scramble and matching pairs.
- Layout: `DictionaryUI.LearningHubDesign` = Classic, Mastery Dashboard or Cognitive Canvas.
- Text in overflowing groups stacks instead of shrinking.

### 5.13 Read aloud (`ReadAloud.cpp`)

- **Speech:** uses WinRT `SpeechSynthesizer`, falling back to SAPI 5. The voice is `ReadAloudVoiceId` and the speed `ReadAloudSpeed`.
- **Start points:** the top of the page, the cursor or the selection.
- **Controls:** pause, continue and stop. A playback bar is drawn over the canvas, and the toolbar button is optional (`ToolbarShowReadAloud`).
- **Highlighting:** the spoken text is mapped back to glyph positions, and the current word is highlighted.

### 5.14 AI chat sidebar (`AIChatPanel.cpp`, `AIChatCommon.cpp`, provider files)

- **Panel:**
  - One sidebar panel (`AIChatSidebarDx`) for all providers, drawn in WebView2. The chat page and `marked.min.js` are served from a virtual host.
  - Comics, image folders, single images and DjVu have no chat.
- **Providers:** each provider is an `AIChatProvider` that runs a local agent CLI as a child process and parses its streaming JSON output.
  - **Claude Code**: runs `claude`. Its session history is in `~/.claude/projects/<encoded cwd>`, where the encoding removes `:`, turns `\` and `_` into `-`, and drops a trailing `-`.
  - **OpenAI Codex**: the model catalog comes from the Codex app-server API, falling back to a built-in list. Sessions are in `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, matched by cwd.
  - **Grok Build**: models come from `grok models`. Sessions are in `~/.grok/sessions/<url-encoded dir>/`.
  - **Antigravity**: models come from `agy models`, which prints a status line and then `id<TAB>name` lines.
- **Settings per provider** (`ClaudeCode.*`, `CodexBuild.*`, `GrokBuild.*`, `AntiGravity.*`; five fields each): model, extra models, effort, skip permissions, and the panel background colour. For example, `ClaudeCode.SkipPermissions` adds `--dangerously-skip-permissions`.
- **Context:** the document text or path is passed as the agent's context. Sessions can be resumed.
- **Debugging:** a bounded in-memory log of recent traffic is kept for post-mortem debugging.

### 5.15 Study export (`StudyExport.cpp`)

- **Content:** highlights, notes and vocabulary, with page references.
- **Formats:** Markdown, plain text, Typst, HTML, Word (`.docx`), JSON and CSV. Text and Markdown can use Skim-style references.
- **CSV safety:** cells that a spreadsheet would treat as formulas are escaped.

### 5.16 Markdown, HTML and text

- **Markdown view:**
  - Rendered with cmark-gfm in WebView2. The TOC comes from headings using GitHub slug rules. Folders are browsed up to two levels deep (`MarkdownToc`). Mermaid diagrams are supported.
  - Settings: `MarkdownUI.*`.
    - `UseFixedPageUI` renders with MuPDF and cmark-gfm instead of WebView2.
    - Fonts: `FontName`, `MathFontName`.
    - Layout: `ReadingWidth`, `LeftMargin`, `RightMargin`.
  - Zoom affects content only. Ctrl+wheel, the toolbar, the default zoom and the remembered zoom stay in sync.
- **Markdown editing** (`TextViewWnd.cpp`):
  - An Edit Markdown toolbar with 13 tools: bold, italic, underline, heading, list, link, task, colour, align, code, highlight, callout and strike.
  - Shortcuts: Ctrl+B/I/U/K/S.
  - Tools wrap the selection or insert a placeholder.
- **Virtual URLs:** URLs stay percent-encoded end to end (issue #6140), and `#fragment` is kept for heading scrolls.
- **Help manual:** the built-in manual is Markdown, rendered on demand (`HelpTheme`, `HelpWindowPos`).

### 5.17 Ebooks, CHM, comics, images

- **EBookUI:** font name and size, layout width, margins, and whether to use the fixed-page UI for ebooks. `EbookSettingsDialog` edits these.
- **ChmUI:** whether to use the fixed-page UI for CHM.
- **ComicBookUI:** cover as a single page, manga mode, and fit/scaling.
- **ImageUI:** background, interpolation and similar options.
- **Comics:** read from CBZ/CBR/CB7/CBT archives in natural sort order. ComicInfo.xml provides metadata.

### 5.18 LaTeX / SyncTeX

- **Inverse search:** double-click a point in the PDF to run `InverseSearchCmdLine`, with `%f` (file), `%l` (line) and `%c` (column). `InverseSearchDialog` presets editors. `EnableTeXEnhancements` shows the setting in the Options dialog.
- **Forward search:** DDE `[ForwardSearch]` or `-forward-search` highlights the target rectangle. Its look is set by `ForwardSearch.*` (offset, width, colour, permanent).
- **Parser:** `PdfSync.cpp` reads `.synctex(.gz)` and the older `.pdfsync`.

### 5.19 Settings UI

- **Settings dialog** (`SettingsDialog.cpp`): main options, appearance, fonts, sizes, pen settings, the dictionary, and import/export/backups.
- **Advanced settings dialog** (`AdvancedSettingsDialog.cpp`): a searchable tree of every advanced setting with its documentation.
- **Editing settings directly:**
  - The command palette's `=` mode sets a setting directly, for example `=ZoomIncrement = 25`.
  - "Advanced settings…" also opens the settings text file in an editor. The app reloads the file when it changes on disk.
- **Live preview:** with `LiveAppearancePreview` on, appearance changes apply immediately, and Cancel restores the saved values.
- **Export and import:**
  - Export writes `SumatraPDFEnhanced-settings-export.txt` in the same format as the settings file. It includes toolbar choices, colour presets, favorites, history and sessions, subject to the privacy settings.
  - Import validates the file before replacing the current settings.
- **Backups:**
  - `SettingsBackupMode` is Automatic or Manual. Automatic mode debounces saves and keeps the 20 most recent copies.
  - `SettingsBackupDirectory` sets the folder; by default, `Backups` next to the settings file.
  - Import first makes a dated copy of the current settings.
  - Settings transfer is not available in restricted mode, and import is disabled in testing instances.

### 5.20 Customisation

- **Toolbar:**
  - Layout: `ToolbarPosition` (top, bottom, left or right), `ToolbarSize` and `ShowToolbar`.
  - Contents: `ToolbarOrder` and `ToolbarHiddenItems`, or a full custom layout (`ToolbarCustomLayout`). The toolbar is drawn as rounded groups.
  - Overflow: items that do not fit move into an overflow menu. The toolbar height stays fixed when annotation controls expand.
  - Custom buttons: `Toolbar` defines buttons that run any command with arguments and use any icon.
- **Shortcuts:**
  - `Shortcuts[]` entries have `Cmd` (a command with optional arguments), `Key` (for example `Ctrl+Shift+P`, or a key sequence) and `Name` (shown in the palette).
  - `ShortcutParse.cpp` parses keys using the generated `gVirtKeysNum`.
  - User shortcuts override the built-in accelerators.
- **Command palette** (`CommandPalette.cpp`, Ctrl+K and Ctrl+Shift+P):
  - Fuzzy search over commands (names and alternative names), open tabs, file history, and settings (`=` prefix).
  - "Debug: …" commands are listed last.
  - It is a draggable window with a captioned title bar.
- **External viewers:** `ExternalViewers[]` adds "Open in …" commands. Each entry has `CommandLine` (with `%1`, `%p`, `%d`), `Name`, `Filter`, `Key` and `Icon`.
- **Translations:** `UiLanguage` selects the language. `Translations.cpp` reads the embedded `translations.txt` (`Tr("…")`), and `ChangeLanguageDialog` changes it.
- **Keyboard help:** `KeyboardHelp` shows a sheet of all shortcuts.

### 5.21 Install, shell integration, updates

- **Installer** (`Installer.cpp`, `InstallerCommon.cpp`, `RegistryInstaller.cpp`):
  - The app exe is also the installer. Its name (`*-install.exe`) or `-install` selects install mode. The payload is embedded.
  - Options: per-user or all users (`-all-users`), install directory, file associations, the search filter, preview handlers and a desktop shortcut. Installation can be silent (`-s`).
- **Uninstaller:** `Uninstaller.cpp`.
- **Shell extensions:**
  - Windows Search IFilter for PDF (`PdfFilter.dll`, `RegistrySearchFilter`).
  - Explorer preview pane and thumbnails (`PdfPreview.dll`, `RegistryPreview`) for PDF, EPUB, CBZ/CBR, DjVu, XPS, TGA, MOBI and FB2.
- **Default viewer:** set through `RegistryAccess`.
- **Portable mode:** settings sit next to the exe when the exe is not in Program Files. The `-appdata` flag changes the folder.
- **Update check** (`UpdateCheck.cpp`, `EnhancedUpdate.cpp`):
  - `CheckForUpdates` turns it on. It uses GitHub releases. `VersionToSkip` and `TimeOfLastUpdateCheck` record state.
  - Each download is checked before installing: digest, architecture, product identity and the four-field version resources. Calendar versions are compared, and legacy versions are migrated. `-update-self-to` performs the switch.
  - `InternetAccess` in restrict.ini blocks it.
- **Crash handling:** a minidump plus a log is written, and crash reports are sent when internet access is allowed. `HangDetector` watches the UI thread for hangs.

### 5.22 Diagnostics and automation

- **Logging:** `-log`, `-log-to-file`, and `-start-perf-log`/`-log-perf-file` (`PerfLog`).
- **`-dbg-control <pipe>`:** a binary request/response protocol for tests (§9.2).
- **Testing flags:** `-for-testing` (new instance, no session restore, no settings save), `-unit-tests` (debug builds) and `-for-ai` (plain diagnostics).
- **Benchmarks and dumps:** `-stress-test`, `-bench`, `-render`, `-extract-text` and `-engine-dump`.
- **SumatraControl** (`SumatraControl.cpp`): an embeddable-control API. UIA providers support accessibility.
- **Plugin mode:** `-plugin <hwnd>` embeds the viewer in a host window, as in old browser plugins.

---

## 6. Settings

### 6.1 Storage, file format and behaviour

**Data folder.** The data folder can be chosen by the user.

- **Portable mode:** the exe is not in an install folder. The bootstrap file `SumatraPDFEnhanced-storage.txt` sits next to the exe.
- **Installed:** the bootstrap file is at `%LOCALAPPDATA%\SumatraPDF Enhanced-config\SumatraPDFEnhanced-storage.txt`.
- **Data folder:** the bootstrap file names it. The default is `%LOCALAPPDATA%\SumatraPDF Enhanced`.
- **Validation:** a chosen folder must be writable, which is tested by creating a temp file in it. If it goes missing, the app falls back to the last folder that worked and tells the user.
- **Moving data:** moves only these entries: `dictionaries\`, `sumatrapdfcache\`, `notes\`, `vocabulary\`, `Screenshots\`, `SumatraPDFEnhanced-settings.txt` and `SumatraPDF-vocabulary.json`. An upstream `SumatraPDF-settings.txt` is renamed to `SumatraPDFEnhanced-settings.txt`.

| File (in the data folder) | Contents |
|---|---|
| `SumatraPDFEnhanced-settings.txt` | all settings in §6.2, plus file history, sessions and favorites |
| `SumatraPDF-vocabulary.json` | vocabulary library and scheduling state |
| `SumatraPDF-learning-guide.txt`, `SumatraPDF-learning-layout.txt` | learning hub guide and layout |
| `dictionaries\` | imported dictionaries and Kaikki packs (`<pack>\<generation>\manifest.json`, `index.bin`, `records.jsonl`) |
| `sumatrapdfcache\` | home-page thumbnails (PNG, keyed by a file hash) |
| `notes\`, `Screenshots\` | user notes and captured screenshots |
| `Backups\` (default `SettingsBackupDirectory`) | dated settings backups; the 20 most recent automatic copies are kept |
| annotation recovery folder | PDF snapshots plus a versioned manifest (§5.8) |
| `SumatraPDF-data\<sha1-prefix>\` (temp) | per-exe-build extracted helper files |

**File format.** A sparse, indentation-free text format (upstream "SquareTree"). Only values that differ from the default are written.

```
# comment
ZoomIncrement = 25
FixedPageUI [
    TextColor = #000000
    InvertColors = false
]
FileStates [
    [
        FilePath = C:\docs\a.pdf
        PageNo = 12
        Zoom = fit width
    ]
    [
        FilePath = C:\docs\b.pdf
    ]
]
Shortcuts [
    [
        Cmd = CmdOpenFile
        Key = Ctrl+Shift+O
    ]
]
```

**Value types.**

- **Bool:** `true`/`false`.
- **Int, Float.**
- **String:** the rest of the line.
- **Color:** `#rrggbb` or `#aarrggbb`. Some colours take a list.
- **Compact:** fixed-size lists written on one line, for example `WindowPos = 10 10 800 600`.
- **IntArray / FloatArray / StringArray:** space-separated.
- **struct:** `Name [ … ]`.
- **array of struct:** `Name [ [ … ] [ … ] ]`.

**Behaviour.**

- The parser ignores unknown keys and keeps defaults for keys that are missing or invalid.
- The file is watched on disk and reloaded when it changes. Some settings take effect only after a restart.
- **Tags in the table:** `internal` means the setting is not documented to users; `not saved` means it is runtime-only; `vX` is the version that introduced it.
- Rows ending in `[]` are fields of each array element. Dotted names are nested struct fields.

### 6.2 Every setting

Generated from `cmd/gen-settings.ts` (488 rows), with the defaults shown after `=`.

| Setting | Type | Default | Tags | Description |
|---|---|---|---|---|
| `DefaultDisplayMode` | String | ` = automatic` |  | default layout of pages. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view, page aspect. page aspect (3.7+): first open of a PDF, XPS, DjVu or PostScript file uses page 1 — taller than wide is continuous + fit width, wider than tall is single page + fit page; a remembered FileState still wins |
| `DefaultZoom` | String | ` = fit page` |  | default zoom. valid values: fit page, fit width, fit height, fit content, fit visible or percent like 100% |
| `DisableJavaScript` | Bool | ` = false` | v3.7 | if true, JavaScript in PDF documents is disabled (e.g. form-field calculations won't run) |
| `AllowExternalImages` | Bool | ` = false` | v3.7 | if true, a PDF may load an image stored in a separate file referenced by name (an external image stream); the file must sit next to the PDF. Off by default for security (matches Acrobat) |
| `EnableTeXEnhancements` | Bool | ` = false` |  | if true, show the SyncTeX inverse search command line in Settings -> Options, so a double-click in the document can jump to the matching line in a LaTeX editor |
| `EscToExit` | Bool | ` = false` |  | if true, Esc key closes SumatraPDF. In presentation or fullscreen mode, Esc leaves that mode first |
| `FullPathInTitle` | Bool | ` = false` | v3.0 | if true, show the full path to the document in the title bar |
| `InverseSearchCmdLine` | String | ` =` |  | pattern used to launch the LaTeX editor when doing inverse search |
| `LazyLoading` | Bool | ` = false` | v3.6 | if true, restoring a session delays loading each document until its tab is selected |
| `MainWindowBackground` | Color | ` = #80fff200` |  | background color of the area around the document, traditionally yellow. Only applies to the Light theme; the default #80fff200 is a marker meaning "use the theme's color", so setting any other value also colorizes the toolbar and sidebars |
| `NoHomeTab` | Bool | ` = false` |  | if true, doesn't open Home tab |
| `HomePageSortByFrequentlyRead` | Bool | ` = false` |  | if true, the home page lists documents by how often they've been opened (the pre-3.6 behavior); if false, the most recently opened come first |
| `HomePageViewMode` | String | ` = thumbnails` | v3.7 | valid values: thumbnails, list |
| `HomePageMaxRecentItems` | Int | ` = 30` |  | maximum recent documents shown on the home page, from 1 to 200 |
| `HomePageFixedHeader` | Bool | ` = false` |  | keep the home page header visible while recent documents scroll |
| `HomePageFullPageThumbnails` | Bool | ` = false` |  | show whole document pages instead of cropped home page previews |
| `HomePageThumbnailPadding` | Bool | ` = true` |  | show framed cards and padding around home page thumbnails; false places previews and filenames directly on the home page |
| `TabListVisibleItems` | Int | ` = 10` |  | maximum visible rows in the open-file tab list, from 1 to 50 |
| `ScrollbarWidth` | Int | ` = 30` |  | app scrollbar width in logical pixels, from 8 to 60 |
| `ScrollbarWidthExpanded` | Bool | ` = false` | internal | saved scrollbar width has been expanded once |
| `HomePageThumbnailSize` | Int | ` = 100` |  | home grid preview size in percent, from 75 to 250; Ctrl+wheel also adjusts it |
| `FilePicker` | String | ` = ` | v3.7 | valid values: (empty), os, sumatrapdf |
| `PrinterUI` | String | ` = ` | v3.7 | valid values: (empty), auto, modern, classic |
| `ReloadModifiedDocuments` | Bool | ` = true` | v2.5 | if true, a document will be reloaded automatically whenever it's changed (currently doesn't work for documents shown in the ebook UI) |
| `RememberOpenedFiles` | Bool | ` = true` |  | if true, keep a history of opened documents and their display settings (FileStates); closing a document doesn't remove it from the history. Also required for saving SessionData |
| `RememberStatePerDocument` | Bool | ` = true` |  | if true, store display settings for each document separately (i.e. everything after UseDefaultState in FileStates) |
| `RestoreSession` | Bool | ` = true` |  | if true, documents that were still open when the last window was closed (SessionData) are reopened at startup |
| `ReuseInstance` | Bool | ` = true` |  | if true, open documents in the already running SumatraPDF instead of starting a new one |
| `ShowMenubar` | Bool | ` = true` | v2.5 | if true, show the menu bar (F9 toggles it; the choice is remembered across sessions) |
| `ShowMenubarWithTabs` | Bool | ` = false` | v3.7 | if true, show the menu bar when using tabs (useTabs = true) |
| `ShowPageNumberInTabs` | Bool | ` = false` | v3.7 | if true, show the current page as n/N after the file name on tabs |
| `ShowHomePageReadingProgress` | Bool | ` = true` | v3.7 | if true, show reading progress (n/N, or chapter:page for ebooks) on home page thumbnails and list rows |
| `ShowChaptersInEbooks` | Bool | ` = false` | v3.7 | if true, a document with chapters (EPUB, MOBI) shows the current place as a chapter and a page within that chapter, in the toolbar, Go to Page and the page-info tip. if false, those show one page number for the whole document. the saved position stays a chapter bookmark either way, and next / previous page still cross chapters |
| `ShowTips` | Bool | ` = true` | v3.7 | if true, show tips on the home page |
| `CustomColors` | String | ` =` | internal, v3.7 | up to 13 custom colors for the background color picker, separated by space (e.g. '#ff0000 #00ff00 #0000ff') |
| `ShowToolbar` | Bool | ` = true` | internal | legacy bool for toolbar; if Toolbar is empty, derived as show/hide (internal; use Toolbar instead) |
| `Toolbar` | String | ` = show` | v3.7 | toolbar mode: show (pinned), hide (no toolbar), overlay (toolbar floats over the page, sized to its natural width and centered, only shown when the mouse is near it). if empty, derived from ShowToolbar |
| `ToolbarPosition` | String | ` = top` | v3.7 | where the toolbar is placed: top or bottom (applies to both show and overlay modes) |
| `SearchUIFloating` | Bool | ` = false` | v3.7 | if true, the find UI is a floating, movable window with a results list instead of the compact toolbar overlay |
| `ShowFavorites` | Bool | ` = false` |  | if true, show the sidebar's bottom panel: Favorites, unless SidebarBottomView says otherwise |
| `SidebarBottomView` | String | ` =` | v3.7 | what the sidebar's bottom panel shows: favorites (the default), bookmarks or thumbnails |
| `SortFavoritesByName` | Bool | ` = false` | v3.7 | if true, favorites within each file are sorted alphabetically by name (or page label); if false (the default), they are sorted by page number |
| `ShowToc` | Bool | ` = true` |  | if true, show the table of contents (Bookmarks) sidebar when the document has one |
| `AlwaysShowSidebar` | Bool | ` = false` | v3.7 | if true, every document with bookmarks opens with the Bookmarks sidebar, even one that was closed with it hidden |
| `SidebarOnRight` | Bool | ` = false` | v3.7 | if true, put the bookmarks / favorites sidebar on the right of the window (left is the default; right-to-left UI languages already put it on the right) |
| `SidebarWindowSize` | String | ` = ` | v3.7 | valid values: (empty), keep, grow |
| `ShowLinks` | Bool | ` = false` | v3.6 | if true, draw a blue border around links in the document |
| `HighlightFormFields` | Bool | ` = true` | v3.7 | if true, highlight empty fillable PDF form fields in pale blue so they are easy to find |
| `ClickEdgeToTurnPage` | Bool | ` = false` | v3.7 | if true, a click (not a drag) on the left fifth of the page area goes to the previous page and a click on the right fifth goes to the next page (reversed in manga / right-to-left mode). Links, annotations and presentation-mode clicks are unchanged |
| `DisableLinks` | Bool | ` = false` | v3.7 | if true, document links are ignored so you can select and read (useful for drawings with many links); if false, clicking a link follows it |
| `ExplorerQuickLook` | Bool | ` = false` | v3.7 | if true, Space in File Explorer (or on the desktop) previews the selected file in a popup window, like macOS Quick Look. Esc or Space closes it; Left / Right open the previous / next file in the folder. Starts a small background helper at logon so it works even when SumatraPDF is not open |
| `RememberViewOffsetOnPageTurn` | Bool | ` = false` | v3.7 | if true, next/previous page keeps the same view position on the page instead of jumping to the top (useful when zoomed in on similarly sized pages) |
| `MouseWheelTurnsPage` | Bool | ` = false` | v3.7 | if true, one mouse-wheel notch goes to the next / previous page instead of scrolling; combine with RememberViewOffsetOnPageTurn to read zoomed-in pages without touching the keyboard. Alt + wheel still scrolls, Shift + wheel scrolls horizontally and Ctrl + wheel zooms |
| `ScrollEdgeTurnsPage` | Bool | ` = true` | v3.7 | if true, in single page / facing / book view, scrolling past the top or bottom of a zoomed-in page goes to the previous / next page; if false, scrolling stops at the edge and the page is changed only by the keyboard, toolbar or scrollbar. A page that fits the window has nothing to scroll, so a wheel notch turns it either way |
| `ShowDocumentFocusIndicator` | Bool | ` = false` | v3.7 | if true, draw a focus ring around the document when it has keyboard focus (Tab to the page area) |
| `ShowAnnotationNotification` | Bool | ` = true` | v3.7 | if true, show a tip when hovering an annotation (e.g. "Highlight annotation. Ctrl+click to edit.") |
| `ShowFileNavigateHint` | Bool | ` = true` | v3.7 | if true, at the end of a document show a hint to open the next file in the folder. Closing the hint sets it to false |
| `ShowAnnotationAuthorInTooltip` | Bool | ` = false` | v3.7 | if true, show the author at the bottom of an annotation tooltip as "Author: <author>" |
| `ShowTocPageNumbers` | Bool | ` = true` | v3.7 | if true, show page numbers (labels) right-aligned on bookmark / table-of-contents entries |
| `AutoGenerateTOC` | Bool | ` = false` | v3.7 | if true, a PDF without an outline gets a table of contents built from numbered headings in its text (Generate Table Of Contents command does it on demand) |
| `ShowStartPage` | Bool | ` = true` |  | if true, show a list of frequently read documents when no document is loaded |
| `SidebarDx` | Int | ` = 0` | internal | width of the favorites / bookmarks sidebar in screen pixels, as last resized (0 means the default) |
| `Scrollbars` | String | ` = windows` | v3.7 | scrollbar mode: windows (standard Windows scrollbar), smart (overlay scrollbar with auto-hide), overlay (always visible overlay scrollbar), hidden (no scrollbars) |
| `ScrollbarInSinglePage` | Bool | ` = false` | v3.6 | if true, show a scrollbar in single page mode as well |
| `SmoothScroll` | Bool | ` = true` | v3.6 | if true, smooth mouse-wheel and arrow-key scrolling (exponential chase of the target; continuous input stays fluid) |
| `ScrollLineAmount` | Int | ` = 16` | v3.7 | distance, in screen pixels at 96 DPI, scrolled by an arrow-key press or one mouse-wheel line; values below 1 use 16 |
| `SaveMemory` | Int | ` = 50` | v3.7 | how hard to free unused page and image caches to save RAM (0 to 100). 0 keeps them until an allocation fails; 100 drops them as soon as a page is off-screen |
| `PaddingAfterLastPage` | Bool | ` = false` | v3.7 | if true, continuous view has extra scroll room after the last page so you can scroll the end of the document to the top of the window |
| `IgnoreDestinationZoom` | Bool | ` = false` | v3.7 | if true, going to a destination (clicking a bookmark or a link inside the document) keeps the current zoom instead of applying the zoom the destination asks for; it still goes to the page and the position. Same as Adobe Reader's 'forbid the change of the current zoom factor during execution of Go to Destination actions' |
| `HighlightLinkDestination` | Bool | ` = false` | v3.7 | if true, following an internal link or bookmark flashes a highlight at the destination so you can see where you landed (a bibliography entry, figure, or named destination). The color and fade match ForwardSearch. Off when the destination is only a page with no position |
| `CitationHoverDelay` | Int | ` = 300` | v3.7 | how long an internal-document link has to be hovered, in milliseconds, before a popup rendering the destination region (citation entry, figure, footnote) appears. -1 (the disabled value) disables the popup; default 300 enables previews |
| `ReadAloudVoiceId` | String | ` =` | v3.7 | voice id for Read Aloud text-to-speech; empty or unset means system default. Voice ids match those used internally by the Read Aloud Voice menu (WinRT voice id or SAPI token id) |
| `ReadAloudSpeed` | Float | ` = 1` | v3.7 | playback speed multiplier for Read Aloud text-to-speech (0.5 .. 3.0), 1 is normal speed; can also be changed from the Read Aloud playback bar |
| `ReadingAutoScrollSpeed` | Float | ` = 40` | v3.7 | pixels per second for Automatically Scroll (View menu / Ctrl+Shift+H). 8 to 320; also changed from the auto-scroll bar and the arrow keys while scrolling |
| `ReadingBar` | struct | | v3.7 | reading bar (View menu): a horizontal band on the page to keep your place. Highlight fills the band; Invert dims everything else |
| `ReadingBar.Background` | Color | ` = #66ffe082` | v3.7 | fill of the reading bar in highlight mode. #aarrggbb sets opacity (00 = transparent, FF = opaque); #rrggbb is fully opaque |
| `ReadingBar.Invert` | Bool | ` = false` | v3.7 | if true, dim the page except the reading bar (screen mask); if false, draw a colored highlight band |
| `ReadingBar.Height` | Int | ` = 0` | v3.7 | height of the reading bar in pixels at 96 DPI; 0 uses a default of about three lines. dragging the top or bottom edge of the bar also changes this |
| `FastScrollOverScrollbar` | Bool | ` = false` | v3.6 | if true, mouse wheel scrolling is faster when mouse is over a scrollbar |
| `PreventSleepInFullscreen` | Bool | ` = true` |  | if true, prevents the screen from turning off when in fullscreen or presentation mode |
| `MinTabWidth` | Int | ` = 100` |  | minimum tab width in logical pixels; overflow scrolls |
| `TabWidth` | Int | ` = 300` |  | maximum width of a single tab, in pixels at 100% display scaling (at least 60) |
| `Theme` | String | ` = Sumatra Light` | v3.5 | valid themes: Sumatra Light/Dark, Modern Slate/Blue/Green/Purple/Amber Light/Dark, System |
| `HelpTheme` | String | ` = app` | v3.7 | color theme of the in-app manual (F1): app (follow the app's theme), light or dark. The switch in the manual's top-right corner changes it |
| `LastLightTheme` | String | ` = ` | internal, v3.7 | the light theme the light/dark toggle and the System theme switch to |
| `LastDarkTheme` | String | ` = ` | internal, v3.7 | the dark theme the light/dark toggle and the System theme switch to |
| `DocumentColorsFollowTheme` | String | ` = off` | v3.7 | how MuPDF-rendered documents (PDF, XPS, DjVu, EPUB, MOBI, FB2, CBZ, images, etc.) use UI / FixedPageUI colors for the page. Values: off (document's own colors; default); smart (recolor text and page background, keep photos/images as-is — best for dark reading); legacy (also recolor images; pre-3.7 invert-style). Does not change menus/toolbars — use Theme for UI chrome. Settings / Theme and the CmdSetDocumentColorsFollowTheme command set all three values. Shift+I (Invert Colors) is separate: it swaps the page colors for the session whatever this is set to |
| `TocDy` | Int | ` = 0` | internal | if both the favorites and the bookmarks part of the sidebar are visible, this is the height of the bookmarks (table of contents) part, in screen pixels |
| `ToolbarCustomLayout` | String | ` = ` | v3.7 | the toolbar's built-in buttons, in the order you want them, e.g. CmdOpenFile CmdPrint PageInfo | CmdFindFirst. Leave a button out to hide it. | is a separator and PageInfo is the page number box. Empty (the default) means the standard layout. Buttons you added yourself (see Shortcuts) still come last |
| `ToolbarOrder` | String | ` = ` | internal | saved order of toolbar command icons |
| `ToolbarAnnotationOrder` | String | ` = ` | internal | saved order of annotation toolbar command icons |
| `ToolbarHiddenItems` | String | ` = ` |  | hidden toolbar command names separated by spaces; PageInfo hides the page number group. Pinned annotation presets are independent. Use the toolbar dropdown to show or hide items |
| `ToolbarShowReadAloud` | Bool | ` = false` | v3.7 | if true, the toolbar has a Read Aloud button (with a drop-down for voice, speed and what to read). Read Aloud is still reachable from the Read Aloud menu when this is false |
| `AnnotationToolbarColumns` | Int | ` = 1` |  | number of icon columns in the side toolbar in Annotation Mode (1-3) |
| `AnnotationToolbarStyle` | Int | ` = 1` |  | layout of the vertical Annotation Mode toolbar with 2 or 3 columns: 1 = grouped (default), 2 = continuous rail. Does not change single-column or horizontal toolbars |
| `AnnotationModeIconStyle` | Int | ` = 1` |  | Annotation Mode icon pen style: 1 = solid, 2 = fine tip |
| `AnnotationPageNumberStyle` | Int | ` = 1` |  | page number layout in the vertical Annotation Mode toolbar: 1 = short separator (default), 2 = one divided box, 3 = two stacked boxes. The current page is above the last page |
| `AnnotationThreeColumnPageStyle` | Int | ` = 1` |  | page number layout only in the three-column vertical Annotation Mode toolbar: 1 = current stacked layout (default), 2 = full-width horizontal page row, 3 = compact horizontal page field between Previous and Next. Other toolbar layouts are unchanged |
| `ToolbarSize` | Int | ` = 18` | v3.4 | size of the toolbar icons in pixels at 100% display scaling (8-64); the toolbar itself is a few pixels taller |
| `TreeFontName` | String | ` = automatic` |  | font name for bookmarks and favorites tree views. automatic means Windows default |
| `TreeFontSize` | Int | ` = 0` | v3.3 | font size for bookmarks and favorites tree views, in pixels; 0 means the Windows default. Not scaled by the display scaling |
| `PinIconStyle` | Int | ` = 10` |  | pin icon design: 10 = soft outline (default), 3 = solid, 4 = round head; all use the same diagonal angle |
| `FileTypeIconStyle` | Int | ` = 3` |  | Windows file icons: 3 = outlined pages (included, default), 1 = folded pages, 2 = color covers. Alternatives download from GitHub after confirmation. Applies to this app's file associations; does not change default apps |
| `LiveAppearancePreview` | Bool | ` = false` |  | preview appearance colors and icon styles while choosing them in settings; Cancel restores the saved appearance. Does not download assets or change Windows associations before saving |
| `SettingsBackupMode` | String | ` = automatic` |  | settings backups: automatic backs up changed saved preferences in the background and keeps the latest 20 copies; manual uses Back Up Now |
| `SettingsBackupDirectory` | String | ` = ` |  | folder for settings backups; empty uses the Backups folder beside the settings file |
| `ColorPickerIconStyle` | Int | ` = 8` |  | color picker icon design: 8 = color tiles (default), 1 = classic palette, 2 = soft palette, 6 = palette and dropper, 5 = color wheel |
| `PenCursorStyle` | Int | ` = 11` |  | drawing cursor design: 11 = rollerball, 12 = drafting pen, 9 = lacquer pen with conical tip |
| `PenCursorSize` | Int | ` = 20` |  | drawing cursor size in logical pixels, from 0 to 100; 0 hides the drawing cursor; scales with monitor DPI, independently of interface and toolbar size |
| `PenCursorAccent` | Color | ` = #2675e8` |  | accent color of the drawing cursor; independent of annotation ink color |
| `ActiveTabStyle` | Int | ` = 3` |  | active tab design: 3 = outlined (default), 1 = solid color, 4 = inset |
| `ActiveTabAccent` | Color | ` = #2675e8` |  | active tab accent color; a file's Change Tab Color overrides it |
| `InterfaceScale` | Int | ` = 100` |  | overall interface scale, from 50 to 250 percent; document zoom is independent |
| `UIFontFamily` | String | ` = system` |  | interface font: system, Manrope, Pretendard Std or Public Sans |
| `UIFontSize` | Int | ` = 0` | v3.6 | overrides the font size used for menus, toolbar and dialogs, in pixels; 0 means the Windows default. Not scaled by the display scaling |
| `DisableAntiAlias` | Bool | ` = false` | v3.6 | if true, render MuPDF-based documents (PDF, XPS, DjVu, EPUB etc.) without anti-aliasing, giving sharper but jagged edges |
| `EngineeringDrawingEnhance` | String | ` = off` | v3.7 | CAD/engineering PDF line rendering: off, auto (enhance if a CAD drawing is detected) or on |
| `DisableAutoLinks` | Bool | ` = false` | v3.7 | if true, disables auto-linking of URLs and email addresses found in PDF text |
| `UseSysColors` | Bool | ` = false` |  | if true, use the Windows system colors for the document background and text. Overrides other color settings |
| `UseTabs` | Bool | ` = true` | v3.0 | if true, documents are opened in tabs instead of new windows |
| `SelectionToolbar` | Bool | ` = true` | v3.7 | if true, a small floating toolbar with selection actions (copy, read aloud, highlight etc.) pops up after selecting text. Set to false to disable it |
| `SelectionToolbarLayout` | String | ` = ` | v3.7 | which built-in buttons the selection toolbar has and in what order, e.g. CmdCopySelection | CmdCreateAnnotHighlight. | or Separator inserts a separator. Leave a button out to hide it. Empty (the default) is the standard set. SelectionHandlers with SelectToolbarNameOrSvg still come last |
| `TabsMru` | Bool | ` = false` | v3.7 | if true, Ctrl+Tab and Ctrl+Shift+Tab show the tab switcher in most recently used order instead of tab-strip order |
| `CtrlTabSimple` | Bool | ` = false` | v3.7 | if true, Ctrl+Tab and Ctrl+Shift+Tab immediately switch to the next / previous tab in tab-strip order (the behavior before version 3.6) instead of showing the tab switcher |
| `ZoomLevels` | FloatArray | ` = ` |  | sequence of zoom levels when zooming in/out; values must lie between 8.33 and 1000000 (the largest one becomes the maximum zoom, which is 6400 by default) |
| `ZoomLevelsCmdIds` | IntArray | ` = ` | not saved | command id assigned to each entry of ZoomLevels |
| `ZoomIncrement` | Float | ` = 0` |  | how much a single zoom in / zoom out step changes the zoom, as a percentage of the current zoom level. If 0 or negative, zooming steps through ZoomLevels instead |
| `FixedPageUI` | struct | |  | customization options for PDF, XPS, DjVu and PostScript UI |
| `FixedPageUI.Grayscale` | Bool | ` = false` | v3.7 | if true, render document pages in grayscale. Toggle with CmdToggleGrayscale |
| `FixedPageUI.TextColor` | Color | ` = #000000` |  | color used instead of black for the document's text |
| `FixedPageUI.BackgroundColor` | Color | ` = #ffffff` |  | color used instead of white for the document's page background |
| `FixedPageUI.SelectionColor` | Color | ` = #ffff00` | v2.4 | color value for the text selection rectangle (also used to highlight found text). Use an #aarrggbb value to control opacity: a smaller alpha (e.g. #40ffff00) makes the selection more transparent so the selected text stays crisp; #rrggbb uses the default opacity |
| `FixedPageUI.WindowMargin` | compact struct | |  | top, right, bottom and left margin (in that order) between window and document |
| `FixedPageUI.WindowMargin.Top` | Int | ` = 2` |  | space between the top of the window and the document, in pixels at 100% display scaling |
| `FixedPageUI.WindowMargin.Right` | Int | ` = 4` |  | space between the right of the window and the document, in pixels at 100% display scaling |
| `FixedPageUI.WindowMargin.Bottom` | Int | ` = 2` |  | space between the bottom of the window and the document, in pixels at 100% display scaling |
| `FixedPageUI.WindowMargin.Left` | Int | ` = 4` |  | space between the left of the window and the document, in pixels at 100% display scaling |
| `FixedPageUI.PageSpacing` | compact struct | |  | horizontal and vertical gap between pages (between columns in facing/book view, between rows in continuous view) |
| `FixedPageUI.PageSpacing.Dx` | Int | ` = 4` |  | horizontal gap between two pages, in pixels at 100% display scaling |
| `FixedPageUI.PageSpacing.Dy` | Int | ` = 4` |  | vertical gap between two pages, in pixels at 100% display scaling |
| `FixedPageUI.GradientColors` | ColorArray | ` =` |  | experimental: instead of a single background color, fade through these colors from the top of the document to the bottom (stops are spread evenly, at most 3 colors). The shifting background is meant to give a subconscious sense of reading progress. Suggested values: #2828aa #28aa28 #aa2828 |
| `FixedPageUI.WindowBgCol` | Color | ` = ` | v3.7 | if given, sets the canvas background color for PDF files |
| `FixedPageUI.PageGrid` | struct | | v3.7 | measurement grid overlay (View > Page Grid). Spacing and style are saved; showing the grid is session-only |
| `FixedPageUI.PageGrid.Width` | Float | ` = 72` | v3.7 | distance between major vertical grid lines, in PDF points (1/72 inch) |
| `FixedPageUI.PageGrid.Height` | Float | ` = 72` | v3.7 | distance between major horizontal grid lines, in PDF points |
| `FixedPageUI.PageGrid.Subdivisions` | Int | ` = 4` | v3.7 | minor lines per major cell |
| `FixedPageUI.PageGrid.OffsetX` | Float | ` = 0` | v3.7 | horizontal origin offset from the left of the page, in PDF points |
| `FixedPageUI.PageGrid.OffsetY` | Float | ` = 0` | v3.7 | vertical origin offset from the bottom of the page, in PDF points |
| `FixedPageUI.PageGrid.Color` | Color | ` = #8080ff` | v3.7 | color of the page grid overlay |
| `FixedPageUI.PageGrid.Style` | String | ` = dots` | v3.7 | grid overlay style: dots (marks at intersections), dotted (dotted lines), or solid |
| `FixedPageUI.PageGrid.Units` | String | ` = in` | v3.7 | units shown in the Configure Page Grid dialog: pt, in, mm, or cm |
| `EBookUI` | struct | |  | customization options for the ebook UI (EPUB, MOBI, FB2, PDB and plain text) |
| `EBookUI.FontName` | String | ` = ` | v3.7 | default font family for ebooks (e.g. Segoe UI, Georgia, Microsoft YaHei). empty uses the engine default (typically a serif). applied as user CSS with !important, which beats the document's own font-family even when it comes from an inline style attribute; leave empty to keep the publisher's fonts. wrapping quotes are stripped. a name that can't be loaded is reported with a notification when the document opens |
| `EBookUI.FontSize` | Float | ` = 0` |  | font size in points; 0 means the default (8.0) |
| `EBookUI.Margin` | FloatArray | ` =` | v3.7 | white space around the text, in points (not screen pixels), like LayoutDx. one number sets all four sides, two are top/bottom and left/right, four are top, right, bottom, left - the same order as in CSS. empty keeps the default (3 em above and below, 2 em left and right, so it follows the font size); 0 leaves no margin at all. each value can be up to 200 |
| `EBookUI.LineSpacing` | Float | ` = 0` | v3.7 | line-height multiplier for ebook text (e.g. 1.5); 0 keeps the document or engine default. values from 0.5 to 5 are accepted |
| `EBookUI.LayoutDx` | Float | ` = 0` |  | width of the page the ebook is laid out into, in points (not screen pixels); 0 means the default (420) |
| `EBookUI.LayoutDy` | Float | ` = 0` |  | height of the page the ebook is laid out into, in points (not screen pixels); 0 derives it from the window's shape when the document is opened, so Fit Width shows a whole page |
| `EBookUI.IgnoreDocumentCSS` | Bool | ` = false` |  | if true, the CSS in the ebook is ignored and only CustomCSS applies |
| `EBookUI.CustomCSS` | String | ` =` |  | additional CSS applied to ebooks; set IgnoreDocumentCSS = true if the document's own CSS overrides it |
| `EBookUI.WindowBgCol` | Color | ` = ` | v3.7 | if given, sets the canvas background color for ebook documents (epub, mobi etc.) |
| `EBookUI.DefaultDisplayMode` | String | ` = ` | v3.7 | default page layout for ebooks; empty uses the global DefaultDisplayMode. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view |
| `ComicBookUI` | struct | |  | customization options for Comic Book UI |
| `ComicBookUI.WindowMargin` | compact struct | |  | top, right, bottom and left margin (in that order) between window and document |
| `ComicBookUI.WindowMargin.Top` | Int | ` = 0` |  | space between the top of the window and the document, in pixels at 100% display scaling |
| `ComicBookUI.WindowMargin.Right` | Int | ` = 0` |  | space between the right of the window and the document, in pixels at 100% display scaling |
| `ComicBookUI.WindowMargin.Bottom` | Int | ` = 0` |  | space between the bottom of the window and the document, in pixels at 100% display scaling |
| `ComicBookUI.WindowMargin.Left` | Int | ` = 0` |  | space between the left of the window and the document, in pixels at 100% display scaling |
| `ComicBookUI.PageSpacing` | compact struct | |  | horizontal and vertical gap between pages (between columns in facing/book view, between rows in continuous view) |
| `ComicBookUI.PageSpacing.Dx` | Int | ` = 4` |  | horizontal gap between two pages, in pixels at 100% display scaling |
| `ComicBookUI.PageSpacing.Dy` | Int | ` = 4` |  | vertical gap between two pages, in pixels at 100% display scaling |
| `ComicBookUI.CbxMangaMode` | Bool | ` = false` |  | if true, documents that don't state their own reading direction default to manga mode, i.e. right to left. A document that states a direction (PDF ViewerPreferences/Direction, or an EPUB with page-progression-direction) is shown the way it asks for |
| `ComicBookUI.WindowBgCol` | Color | ` = ` | v3.7 | if given, sets the canvas background color for comic book files |
| `ComicBookUI.LimitToWindowWidth` | Bool | ` = false` | v3.7 | if true, absolute zoom never makes a page wider than the window (each page is capped at Fit Width). Useful for comics/manga with double-page spreads that are much wider than regular pages (issue #2197) |
| `ComicBookUI.LimitToWindowHeight` | Bool | ` = false` | v3.7 | if true, absolute zoom never makes a page taller than the window (each page is capped at Fit Height) |
| `ComicBookUI.DefaultDisplayMode` | String | ` = ` | v3.7 | default page layout for comic books; empty uses the global DefaultDisplayMode. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view |
| `ComicBookUI.DefaultZoom` | String | ` = ` | v3.7 | default zoom for comic books; empty uses fit page. valid values: fit page, fit width, fit height, fit content, fit visible, shrink to fit or percent like 100% |
| `ComicBookUI.DefaultZoomFloat` | Float | ` = 0` | not saved | value of DefaultZoom for internal usage |
| `ComicBookUI.LandscapeAsSpread` | Bool | ` = true` | v3.7 | if true, in facing and book view a landscape page (wider than tall) occupies the whole two-page row instead of pairing with the next page. For comics that store double-page spreads as one image (issues #1324, #872) |
| `ImageUI` | struct | |  | customization options for image files UI |
| `ImageUI.WindowBgCol` | Color | ` = ` | v3.7 | if given, sets the canvas background color for image files |
| `ImageUI.DefaultZoom` | String | ` = shrink to fit` | v3.7 | default zoom for image files. valid values: fit page, fit width, fit height, fit content, fit visible, shrink to fit or percent like 100% |
| `ImageUI.DefaultZoomFloat` | Float | ` = 0` | not saved | value of DefaultZoom for internal usage |
| `ImageUI.LimitToWindowWidth` | Bool | ` = false` | v3.7 | if true, absolute zoom never makes a page wider than the window (each page is capped at Fit Width). Useful for image folders with mixed aspect ratios (issue #2197) |
| `ImageUI.LimitToWindowHeight` | Bool | ` = false` | v3.7 | if true, absolute zoom never makes a page taller than the window (each page is capped at Fit Height) |
| `ImageUI.LandscapeAsSpread` | Bool | ` = true` | v3.7 | if true, in facing and book view a landscape page (wider than tall) occupies the whole two-page row instead of pairing with the next page (issues #1324, #872) |
| `ChmUI` | struct | |  | customization options for CHM UI. UseFixedPageUI switches to the PDF-style view; FontName applies to that view |
| `ChmUI.UseFixedPageUI` | Bool | ` = false` |  | if true, the UI used for PDF documents will be used for CHM documents as well |
| `ChmUI.FontName` | String | ` =` | v3.7 | font family for the CHM fixed-page view (e.g. Segoe UI, Georgia, Microsoft YaHei). empty uses EBookUI.FontName or the engine default. overrides fonts specified by the document; wrapping quotes are stripped |
| `MarkdownUI` | struct | |  | customization options for Markdown UI. If UseFixedPageUI is true, MuPDF is used; otherwise WebView2 browser view is used when available |
| `MarkdownUI.UseFixedPageUI` | Bool | ` = false` |  | if true, use MuPDF (cmark-gfm) to render markdown; if false, use WebView2 browser view when available |
| `MarkdownUI.FontName` | String | ` = ` |  | Markdown text font; empty follows the interface font |
| `MarkdownUI.MathFontName` | String | ` = ` |  | Markdown MathML font; empty uses the companion of the text font |
| `MarkdownUI.ReadingWidth` | Float | ` = 0` |  | browser Markdown reading width percentage; 0 preserves the supplied layout |
| `MarkdownUI.LeftMargin` | Float | ` = 10` |  | browser Markdown left margin percentage when a custom reading width is used |
| `MarkdownUI.RightMargin` | Float | ` = 10` |  | browser Markdown right margin percentage; width plus margins must total 100 |
| `DictionaryUI` | struct | |  | dictionary reading appearance |
| `DictionaryUI.FontName` | String | ` = ` |  | dictionary reading font; empty follows the interface font; accepts any installed Windows font |
| `DictionaryUI.LineSpacing` | Float | ` = 1.15` |  | dictionary line-height multiplier; greater than 0 and at most 100 |
| `DictionaryUI.FontScale` | Float | ` = 100` |  | dictionary text scale as a percentage of interface text, 50 through 300 |
| `DictionaryUI.LearningHubScale` | Float | ` = 100` |  | Learning Hub text scale as a percentage of interface text, 50 through 300 |
| `HtmlUI` | struct | | v3.7 | customization options for HTML UI. If UseFixedPageUI is true, MuPDF is used; otherwise WebView2 browser view is used when available |
| `HtmlUI.UseFixedPageUI` | Bool | ` = false` |  | if true, use MuPDF to render HTML; if false, use WebView2 browser view when available |
| `ClaudeCode` | struct | | v3.7 | settings for the Claude Code chat sidebar |
| `ClaudeCode.Model` | String | ` = sonnet` |  | Claude model alias for --model (e.g. sonnet, opus, haiku); uses opus if not in the model list |
| `ClaudeCode.Models` | String | ` = ` |  | extra Claude model aliases for the dropdown, comma-separated; documented Claude Code aliases are always included |
| `ClaudeCode.Effort` | Int | ` = 1` |  | Claude effort level: 0=Low, 1=Medium, 2=High, 3=Max |
| `ClaudeCode.SkipPermissions` | Bool | ` = false` |  | if true, pass --dangerously-skip-permissions to Claude Code |
| `ClaudeCode.BgColor` | Color | ` = #ffffff` |  | background color of the Claude Code chat panel |
| `GrokBuild` | struct | | v3.7 | settings for the Grok Build chat sidebar |
| `GrokBuild.Model` | String | ` = grok-4.5` |  | Grok model ID for --model (e.g. grok-4.5) |
| `GrokBuild.Models` | String | ` = ` |  | extra Grok model IDs for the dropdown, comma-separated; used in addition to models reported by Grok |
| `GrokBuild.Effort` | Int | ` = 1` |  | Grok effort level: 0=Low, 1=Medium, 2=High, 3=XHigh, 4=Max |
| `GrokBuild.AlwaysApprove` | Bool | ` = false` |  | if true, pass --always-approve to Grok Build (auto-approve tool executions) |
| `GrokBuild.BgColor` | Color | ` = #ffffff` |  | background color of the Grok Build chat panel |
| `CodexBuild` | struct | | v3.7 | settings for the OpenAI Codex chat sidebar |
| `CodexBuild.Model` | String | ` = gpt-5.5` |  | Codex model ID for -m (e.g. gpt-5.5, gpt-5.4, o3) |
| `CodexBuild.Models` | String | ` = ` |  | extra Codex model IDs for the dropdown, comma-separated; used in addition to models reported by Codex |
| `CodexBuild.Sandbox` | Int | ` = 1` |  | Codex sandbox mode: 0=read-only, 1=workspace-write, 2=danger-full-access |
| `CodexBuild.SkipSandbox` | Bool | ` = false` |  | if true, pass --dangerously-bypass-approvals-and-sandbox to Codex |
| `CodexBuild.BgColor` | Color | ` = #ffffff` |  | background color of the OpenAI Codex chat panel |
| `AntiGravity` | struct | | v3.7 | settings for the Antigravity chat sidebar |
| `AntiGravity.Model` | String | ` = gemini-3.8-flash-medium` |  | Antigravity model ID for --model (e.g. gemini-3.8-flash-medium) |
| `AntiGravity.Models` | String | ` = ` |  | extra Antigravity model IDs for the dropdown, comma-separated |
| `AntiGravity.Effort` | Int | ` = 1` |  | Antigravity effort level: 0=Low, 1=Medium, 2=High, 3=Max |
| `AntiGravity.AutoApprove` | Bool | ` = true` |  | if true, pass --dangerously-skip-permissions to Antigravity CLI so it can read the current file etc. in headless print mode (agy cannot prompt for permissions with -p) |
| `AntiGravity.BgColor` | Color | ` = #ffffff` |  | background color of the Antigravity chat panel |
| `AIChatSidebarDx` | Int | ` = 0` | internal, v3.7 | width of the AI chat sidebar (0 = use default); shared by Claude Code, Grok Build, and OpenAI Codex (internal) |
| `TranslateToLang` | String | ` = ` | internal, v3.7 | remembered destination language for selection translation; empty uses OS UI language |
| `TranslateFromLang` | String | ` = ` | internal, v3.7 | remembered source language for selection translation; empty means Auto |
| `TranslateEngine` | String | ` = ` | internal, v3.7 | remembered engine for Translate Selection: Google, DeepL, Grok Build, Claude Code, OpenAI Codex or Antigravity |
| `Annotations` | struct | | v3.3 | default values for annotations in PDF documents |
| `Annotations.ShapeTool` | Int | ` = 2` |  | last shape selected: 0 = line, 1 = polyline, 2 = rectangle, 3 = ellipse, 4 = polygon, 5 = arrow, 6 = dashed line, 7 = parabola, 8 = negative parabola, 9 = cubic, 10 = negative cubic, 11 = reciprocal, 12 = negative reciprocal, 13 = 2D axes, 14 = 3D axes |
| `Annotations.ShapeCornerRadius` | Float | ` = 0` |  | rectangle corner radius in PDF points; 0 keeps square corners |
| `Annotations.InkBallpoint` | struct | |  | saved settings for this pen type |
| `Annotations.InkBallpoint.Color` | Color | ` = ` |  | saved pen color; empty uses the profile default |
| `Annotations.InkBallpoint.Width` | Float | ` = 0` |  | saved pen width in points; 0 uses the profile default |
| `Annotations.InkBallpoint.Opacity` | Int | ` = 100` |  | saved pen opacity, from 0 to 100 percent |
| `Annotations.InkFountain` | struct | |  | saved settings for this pen type |
| `Annotations.InkFountain.Color` | Color | ` = ` |  | saved pen color; empty uses the profile default |
| `Annotations.InkFountain.Width` | Float | ` = 0` |  | saved pen width in points; 0 uses the profile default |
| `Annotations.InkFountain.Opacity` | Int | ` = 100` |  | saved pen opacity, from 0 to 100 percent |
| `Annotations.InkBrush` | struct | |  | saved settings for this pen type |
| `Annotations.InkBrush.Color` | Color | ` = ` |  | saved pen color; empty uses the profile default |
| `Annotations.InkBrush.Width` | Float | ` = 0` |  | saved pen width in points; 0 uses the profile default |
| `Annotations.InkBrush.Opacity` | Int | ` = 100` |  | saved pen opacity, from 0 to 100 percent |
| `Annotations.InkPencil` | struct | |  | saved settings for this pen type |
| `Annotations.InkPencil.Color` | Color | ` = ` |  | saved pen color; empty uses the profile default |
| `Annotations.InkPencil.Width` | Float | ` = 0` |  | saved pen width in points; 0 uses the profile default |
| `Annotations.InkPencil.Opacity` | Int | ` = 65` |  | saved pen opacity, from 0 to 100 percent |
| `Annotations.InkHighlighter` | struct | |  | saved settings for this pen type |
| `Annotations.InkHighlighter.Color` | Color | ` = ` |  | saved pen color; empty uses the profile default |
| `Annotations.InkHighlighter.Width` | Float | ` = 0` |  | saved pen width in points; 0 uses the profile default |
| `Annotations.InkHighlighter.Opacity` | Int | ` = 40` |  | saved pen opacity, from 0 to 100 percent |
| `Annotations.HighlightColor` | Color | ` = #ffff00` |  | color of newly created highlight annotations. Use an #aarrggbb value to set default opacity (00 = transparent, FF = opaque); #rrggbb is fully opaque |
| `Annotations.UnderlineColor` | Color | ` = #8bf05d` |  | color of newly created underline annotations. #aarrggbb sets default opacity the same way as HighlightColor |
| `Annotations.SquigglyColor` | Color | ` = #f199d2` | v3.5 | color of newly created squiggly underline annotations. #aarrggbb sets default opacity the same way as HighlightColor |
| `Annotations.StrikeOutColor` | Color | ` = #e24745` | v3.5 | color of newly created strike out annotations. #aarrggbb sets default opacity the same way as HighlightColor |
| `Annotations.FreeTextColor` | Color | ` = ` | v3.5 | text color of newly created free text annotations |
| `Annotations.FreeTextBackgroundColor` | Color | ` = ` | v3.6 | background color of newly created free text annotations |
| `Annotations.FreeTextOpacity` | Int | ` = 100` | v3.6 | opacity of free text annotation in percent (0-100); 0 - fully transparent (invisible), 50 - half transparent, 100 - fully opaque |
| `Annotations.FreeTextFontFamily` | String | ` = Helvetica` |  | default font family for new Free Text annotations |
| `Annotations.FreeTextFontStyle` | Int | ` = 0` |  | Free Text style bits: bold=1, italic=2, underline=4 |
| `Annotations.FreeTextSize` | Int | ` = 12` | v3.5 | font size of free text annotations, in points |
| `Annotations.FreeTextBorderWidth` | Int | ` = 1` | v3.5 | border width of free text annotations, in points |
| `Annotations.FreeTextAlignment` | String | ` = left` | v3.7 | how text is aligned in newly created free text annotations (Text Alignment in the compact property row): left, center or right. Right-to-left scripts (Arabic, Hebrew, Persian) want right |
| `Annotations.PresetColors` | String | ` = #ffff00 #8bf05d #99defa #f199d2 #ff9a40 #c680ff #e24745 #ff0000 #0000ff #000000` | v3.7 | colors offered by the drop-down on the annotation toolbar's buttons, separated by space. Picking one sets the color of new annotations of that type. Use the palette add button or right-click a swatch to remove or pin a color |
| `Annotations.TextIconColor` | Color | ` = ` |  | color of newly created text (sticky note) annotations |
| `Annotations.LineColor` | Color | ` = ` | v3.7 | color of newly created line annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.LineInteriorColor` | Color | ` = ` | v3.7 | fill color of newly created line annotations; unset means no fill |
| `Annotations.LineBorderWidth` | Float | ` = 1` |  | default border thickness for line annotations in points, from 0 to 64 |
| `Annotations.LineInteriorOpacity` | Int | ` = 100` | v3.7 | fill opacity of newly created line annotations, from 0 to 100 percent |
| `Annotations.PolyLineColor` | Color | ` = ` | v3.7 | color of newly created polyline annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.PolyLineInteriorColor` | Color | ` = ` | v3.7 | fill color of newly created polyline annotations; unset means no fill |
| `Annotations.PolyLineBorderWidth` | Float | ` = 1` |  | default border thickness for polyline annotations in points, from 0 to 64 |
| `Annotations.PolyLineInteriorOpacity` | Int | ` = 100` | v3.7 | fill opacity of newly created polyline annotations, from 0 to 100 percent |
| `Annotations.SquareColor` | Color | ` = ` | v3.7 | color of newly created square annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.SquareInteriorColor` | Color | ` = ` | v3.7 | fill color of newly created square annotations; unset means no fill |
| `Annotations.SquareBorderWidth` | Float | ` = 1` |  | default border thickness for square annotations in points, from 0 to 64 |
| `Annotations.SquareInteriorOpacity` | Int | ` = 100` | v3.7 | fill opacity of newly created square annotations, from 0 to 100 percent |
| `Annotations.CircleColor` | Color | ` = ` | v3.7 | color of newly created circle annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.CircleInteriorColor` | Color | ` = ` | v3.7 | fill color of newly created circle annotations; unset means no fill |
| `Annotations.CircleBorderWidth` | Float | ` = 1` |  | default border thickness for circle annotations in points, from 0 to 64 |
| `Annotations.CircleInteriorOpacity` | Int | ` = 100` | v3.7 | fill opacity of newly created circle annotations, from 0 to 100 percent |
| `Annotations.PolygonColor` | Color | ` = ` | v3.7 | color of newly created polygon annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.PolygonInteriorColor` | Color | ` = ` | v3.7 | fill color of newly created polygon annotations; unset means no fill |
| `Annotations.PolygonBorderWidth` | Float | ` = 1` |  | default border thickness for polygon annotations in points, from 0 to 64 |
| `Annotations.PolygonInteriorOpacity` | Int | ` = 100` | v3.7 | fill opacity of newly created polygon annotations, from 0 to 100 percent |
| `Annotations.InkColor` | Color | ` = #000000` | v3.7 | color of newly created ink annotations, as #aarrggbb: the alpha is how translucent the stroke is (00 = transparent, FF = opaque), so the color is exactly what ends up on the page |
| `Annotations.InkColors` | String | ` = #000000 #2563eb #dc2626 #facc15 #22c55e #a855f7 #ec4899 #ffffff` | v3.7 | colors offered by the ink button's drop-down, separated by space. Use #aarrggbb values: the alpha is the stroke's opacity. Use the palette add button or right-click a swatch to remove or pin a color |
| `Annotations.InkBorderWidth` | Float | ` = 2` | v3.7 | width of the stroke of new ink annotations, in points |
| `Annotations.StampColor` | Color | ` = ` | v3.7 | color of newly created stamp annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.CaretColor` | Color | ` = ` | v3.7 | color of newly created caret annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.FileAttachmentColor` | Color | ` = ` | v3.7 | color of newly created file attachment annotations. If not set, the PDF engine's default (red) is used |
| `Annotations.TextIconType` | String | ` = ` |  | icon shown for text (sticky note) annotations: comment, help, insert, key, new paragraph, note or paragraph. If not set, note is used |
| `Annotations.DefaultAuthor` | String | ` = ` | v3.4 | author recorded on newly created annotations. If not set, the Windows user name is used; set it to (none) to leave the author out entirely |
| `Annotations.SignatureImage` | String | ` =` | v3.7 | image (e.g. a transparent .png of your signature) that Sign With Image stamps on the page. If not set, or the file is missing, Sign With Image asks for an image |
| `ExternalViewers` | array of ExternalViewer | |  | list of additional external viewers for various file types. See [docs for more information](https://www.sumatrapdfreader.org/docs/Customize-external-viewers) |
| `ExternalViewers[].CommandLine` | String | ` =` |  | command line with which to call the external viewer, may contain %p for page number and "%1" for the file name (add quotation marks around paths containing spaces) |
| `ExternalViewers[].Name` | String | ` =` |  | name of the external viewer to be shown in the menu (implied by CommandLine if missing) |
| `ExternalViewers[].Filter` | String | ` =` |  | optional filter for which file types the menu item is to be shown; separate multiple entries using ';' and don't include any spaces (e.g. *.pdf;*.xps for all PDF and XPS documents) |
| `ExternalViewers[].Key` | String | ` =` | v3.6 | optional: keyboard shortcut e.g. Alt + 7 |
| `ExternalViewers[].ToolbarText` | String | ` =` | v3.7 | if given, shows in toolbar |
| `ExternalViewers[].ToolbarSvgIcon` | String | ` =` | v3.7 | optional SVG icon for toolbar button; if both ToolbarSvgIcon and ToolbarText are set, the icon is used |
| `ForwardSearch` | struct | |  | customization options for how forward search results are shown (used from LaTeX editors) |
| `ForwardSearch.HighlightOffset` | Int | ` = 0` |  | if greater than 0, the forward search result is marked with a bar down the left side of the page instead of highlighting the matched text. The value is how far the bar sits from the left edge of the page, in document units |
| `ForwardSearch.HighlightWidth` | Int | ` = 15` |  | width of that bar in document units (only used when HighlightOffset is greater than 0) |
| `ForwardSearch.HighlightColor` | Color | ` = #6581ff` |  | color used for the forward search highlight |
| `ForwardSearch.HighlightPermanent` | Bool | ` = false` |  | if true, the highlight stays visible until the next mouse click instead of fading away after a few seconds |
| `PrinterDefaults` | struct | |  | these override the default settings in the Print dialog |
| `PrinterDefaults.PrintScale` | String | ` = shrink` |  | default value for scaling (shrink, fit, none) |
| `PrinterDefaults.Collate` | String | ` = default` | v3.7 | default value for collate in the print dialog (default, collate, nocollate) |
| `PrinterDefaults.PrintDpi` | Float | ` = 0` | v3.7 | resolution to assume for the document when printing at original size (e.g. 300 for a 300 dpi scan whose file doesn't say); 0 uses the resolution recorded in the file |
| `Fullscreen` | struct | | v3.7 | options for fullscreen mode |
| `Fullscreen.ShowToolbar` | Bool | ` = false` | internal | legacy bool for fullscreen toolbar; if Fullscreen.Toolbar is empty, derived as show/hide (internal; use Fullscreen.Toolbar instead) |
| `Fullscreen.Toolbar` | String | ` = hide` | v3.7 | toolbar mode in fullscreen: show (pinned), hide (no toolbar), overlay (toolbar floats over the page, only shown when the mouse is near it). if empty, derived from Fullscreen.ShowToolbar |
| `Fullscreen.ShowMenubar` | Bool | ` = false` |  | if true, show the menu bar in fullscreen mode |
| `Fullscreen.DisplayMode` | String | ` = ` | v3.7 | page layout in presentation (Ctrl+L) and windowed fullscreen (Shift+Ctrl+L / F11). empty keeps the current behavior: presentation uses single page, windowed fullscreen keeps the existing layout. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view |
| `SelectionHandlers` | array of SelectionHandler | |  | list of handlers for selected text, shown in context menu when text selection is active. See [docs for more information](https://www.sumatrapdfreader.org/docs/Customize-search-translation-services) |
| `SelectionHandlers[].URL` | String | ` =` |  | url to invoke for the selection. ${selection} is the selected text, ${userlang} the UI language (e.g. 'de'), ${selectionPosition} the selection's screen rect as x,y,dx,dy |
| `SelectionHandlers[].Name` | String | ` =` |  | name shown in context menu |
| `SelectionHandlers[].Key` | String | ` =` | v3.6 | keyboard shortcut |
| `SelectionHandlers[].Exe` | String | ` =` | v3.7 | command line of a program to run instead of opening a URL. Use ${selectionfile} to pass the selection as a temporary utf-8 file, which has no length limit, and ${selectionPosition} for the selection's screen rect (x,y,dx,dy). If set, URL is ignored |
| `SelectionHandlers[].Method` | String | ` = GET` | v3.7 | how to send the selection. GET (default) puts it in the URL, which limits how much text fits. POST sends it in the request body with no length limit and shows the response. POST-VIA-BROWSER submits a form from your browser instead, so the service sees your normal browser session (cookies, logins) |
| `SelectionHandlers[].Body` | String | ` =` | v3.7 | request body for POST / POST-VIA-BROWSER; the same ${selection}, ${selectionjson}, ${userlang} and ${selectionPosition} substitutions apply. If unset, the body is the raw selection |
| `SelectionHandlers[].ContentType` | String | ` =` | v3.7 | value of the Content-Type header for POST. Defaults to 'text/plain; charset=utf-8', which matches the default raw-selection body. Use 'application/json' or 'application/x-www-form-urlencoded' when Body is in that format. Ignored by POST-VIA-BROWSER, which always submits a form |
| `SelectionHandlers[].Headers` | String | ` =` | v3.7 | extra HTTP headers for POST, one per line as 'Name: value' (use \n to separate them in this file). Needed for services that authenticate with an api key, e.g. 'Authorization: Bearer sk-...'. Ignored by POST-VIA-BROWSER. Note that anything you put here is stored in plain text in this settings file |
| `SelectionHandlers[].SelectToolbarNameOrSvg` | String | ` =` | v3.7 | if set, the handler also gets a button on the floating selection toolbar. The value is the button's text, or, if it starts with '<svg', an icon to draw instead. SVG buttons use Name as their tooltip and ToolbarSize as their icon size |
| `SelectionHandlers[].ToolbarText` | String | ` =` | v3.7 | if set, the handler also gets a button on the main toolbar with this label |
| `SelectionHandlers[].ToolbarSvgIcon` | String | ` =` | v3.7 | optional SVG icon for that main-toolbar button; if both ToolbarSvgIcon and ToolbarText are set, the icon is used |
| `TextSnippets` | array of TextSnippet | | v3.7 | predefined text inserted as a free text annotation from the context menu or the command palette |
| `TextSnippets[].Name` | String | ` =` |  | name shown in the context menu and the command palette |
| `TextSnippets[].Text` | String | ` =` |  | text of the free text annotation it inserts; \n starts a new line |
| `TextSnippets[].Key` | String | ` =` |  | keyboard shortcut |
| `Shortcuts` | array of Shortcut | |  | custom keyboard shortcuts |
| `Shortcuts[].Cmd` | String | ` = ` |  | command to run, e.g. CmdOpenFile. See [the list of commands](https://www.sumatrapdfreader.org/docs/Commands) |
| `Shortcuts[].Key` | String | ` = ` |  | keyboard shortcut (e.g. Ctrl-Alt-F) |
| `Shortcuts[].Name` | String | ` =` | v3.6 | name shown in command palette |
| `Shortcuts[].ToolbarText` | String | ` =` | v3.6 | if given, shows in toolbar |
| `Shortcuts[].ToolbarSvgIcon` | String | ` =` | v3.7 | optional SVG icon for toolbar button; if both ToolbarSvgIcon and ToolbarText are set, the icon is used |
| `Shortcuts[].CmdId` | Int | ` = null` | not saved, v3.6 | command id |
| `PenMinWidth` | Float | ` = 0.1` |  | minimum pen width in PDF points |
| `PenMaxWidth` | Float | ` = 16` |  | maximum pen width in PDF points |
| `PenWidthStep` | Float | ` = 0.1` |  | pen width increment in PDF points |
| `LaserLifetimeSeconds` | Float | ` = 2` |  | temporary laser stroke lifetime in seconds (0.1 to 120) |
| `LaserColors` | String | ` = #f44336 #ffc107 #4caf50 #2196f3 #9c27b0 #ffffff` |  | colors offered in the laser pointer palette, separated by spaces |
| `LaserWidth` | Float | ` = 8` |  | laser width in screen pixels at 100% DPI (0.1 to 32), independent of pen width |
| `PinnedAnnotationTools` | array of PinnedAnnotationTool | |  | favorite annotation tool, color and width combinations |
| `PinnedAnnotationTools[].Tool` | String | ` = ballpoint` |  | annotation tool identifier |
| `PinnedAnnotationTools[].Color` | String | ` = #000000` |  | pinned tool color |
| `PinnedAnnotationTools[].Width` | Float | ` = 2` |  | pinned stroke width in PDF points |
| `PinnedAnnotationTools[].ShapeStyle` | Bool | ` = false` |  | whether this preset includes the complete shape appearance |
| `PinnedAnnotationTools[].FillColor` | String | ` = ` |  | pinned shape fill color; empty means no fill |
| `PinnedAnnotationTools[].FillOpacity` | Int | ` = -1` |  | pinned shape fill opacity, 0 through 100; -1 means a legacy border-only preset |
| `PinnedAnnotationTools[].CornerRadius` | Float | ` = 0` |  | pinned rectangle corner radius in PDF points |
| `Themes` | array of Theme | | v3.6 | color themes |
| `Themes[].Name` | String | ` = ` |  | name of the theme, as shown in the Settings / Theme menu |
| `Themes[].TextColor` | Color | ` = ` |  | color of text in menus, toolbar, tabs and sidebars |
| `Themes[].BackgroundColor` | Color | ` = ` |  | background color of the window around the document |
| `Themes[].ControlBackgroundColor` | Color | ` = ` |  | background color of toolbar, tabs, sidebars and dialogs |
| `Themes[].ActiveTabBackgroundColor` | Color | ` = ` | v3.7 | background color of the active tab; if empty, uses ControlBackgroundColor |
| `Themes[].InactiveTabBackgroundColor` | Color | ` = ` | v3.7 | background color of inactive tabs; if empty, derived from ControlBackgroundColor |
| `Themes[].LinkColor` | Color | ` = ` |  | color of clickable links in the UI |
| `Themes[].DisabledTextColor` | Color | ` = ` | v3.7 | color of disabled (grayed out) text; if empty, derived from the colors above |
| `Themes[].DarkerTextColor` | Color | ` = ` | v3.7 | color of secondary / muted text like the page label; if empty, derived from the colors above |
| `Themes[].HotBackgroundColor` | Color | ` = ` | v3.7 | background color of a control the mouse is over; if empty, derived from the colors above |
| `Themes[].EdgeColor` | Color | ` = ` | v3.7 | color of control borders and separators; if empty, derived from the colors above |
| `Themes[].HotEdgeColor` | Color | ` = ` | v3.7 | border color of a control the mouse is over; if empty, derived from the colors above |
| `Themes[].DisabledEdgeColor` | Color | ` = ` | v3.7 | border color of a disabled control; if empty, derived from the colors above |
| `Themes[].ErrorBackgroundColor` | Color | ` = ` | v3.7 | background color of error messages; if empty, derived from the colors above |
| `Themes[].NotificationBackgroundColor` | Color | ` = ` | v3.7 | background color of notification tips; if empty, derived from the colors above |
| `Themes[].NotificationHighlightColor` | Color | ` = ` | v3.7 | background color of a highlighted notification tip; if empty, derived from the colors above |
| `Themes[].NotificationHighlightTextColor` | Color | ` = ` | v3.7 | text color of a highlighted notification tip; if empty, derived from the colors above |
| `Themes[].ColorizeControls` | Bool | ` = false` |  | if true, apply the theme colors to Windows controls and window areas too |
| `TabGroups` | array of TabGroup | | v3.7 | saved groups of tabs |
| `TabGroups[].Name` | String | ` = ` |  | name of the tab group, as shown when restoring it |
| `TabGroups[].TabFiles` | array of TabFile | |  | documents that belong to this tab group |
| `TabGroups[].TabFiles[].Path` | String | ` = ` |  | path of the document |
| `CustomScreenDPI` | Int | ` = 0` | v2.5 | actual resolution of the main screen in DPI, used to show documents at their physical size; if 0 or negative, the resolution reported by Windows is used |
| `DefaultPasswords` | StringArray | ` =` | v2.4 | a whitespace separated list of passwords to try when opening a password protected document (passwords containing spaces must be quoted) |
| `UiLanguage` | String | ` =` |  | [ISO code](langs.html) of the current UI language |
| `VersionToSkip` | String | ` =` |  | SumatraPDF won't offer to update to this version again |
| `WindowState` | Int | ` = 1` |  | default state of the window. 1 is normal, 2 is maximized, 3 is fullscreen, 4 is minimized |
| `WindowPos` | compact struct | |  | default position (x, y) and size (width, height) of the window |
| `WindowPos.X` | Int | ` = 0` |  | x coordinate of the top-left corner, in screen pixels |
| `WindowPos.Y` | Int | ` = 0` |  | y coordinate of the top-left corner, in screen pixels |
| `WindowPos.Dx` | Int | ` = 0` |  | width, in screen pixels |
| `WindowPos.Dy` | Int | ` = 0` |  | height, in screen pixels |
| `SearchUIWindowPos` | compact struct | |  | position/size of the floating find window (see SearchUIFloating) |
| `SearchUIWindowPos.X` | Int | ` = 0` |  | x coordinate of the top-left corner, in screen pixels |
| `SearchUIWindowPos.Y` | Int | ` = 0` |  | y coordinate of the top-left corner, in screen pixels |
| `SearchUIWindowPos.Dx` | Int | ` = 0` |  | width, in screen pixels |
| `SearchUIWindowPos.Dy` | Int | ` = 0` |  | height, in screen pixels |
| `HelpWindowPos` | compact struct | | internal, v3.7 | position/size of the in-app Help: Manual window |
| `HelpWindowPos.X` | Int | ` = 0` |  | x coordinate of the top-left corner, in screen pixels |
| `HelpWindowPos.Y` | Int | ` = 0` |  | y coordinate of the top-left corner, in screen pixels |
| `HelpWindowPos.Dx` | Int | ` = 0` |  | width, in screen pixels |
| `HelpWindowPos.Dy` | Int | ` = 0` |  | height, in screen pixels |
| `FileStates` | array of FileState | |  | history of opened files, most recently used first. A closed file stays here until it drops off the list or the history is cleared |
| `FileStates[].Favorites` | array of Favorite | |  | pages of this document bookmarked in the Favorites menu |
| `FileStates[].Favorites[].Name` | String | ` =` |  | name of this favorite as shown in the menu |
| `FileStates[].Favorites[].PageNo` | String | ` = 1` |  | number of the bookmarked page, or `bm:<bookmark>` for documents with chapters (see PagePosition.cpp) |
| `FileStates[].Favorites[].PageLabel` | String | ` =` |  | label for this page (only present if logical and physical page numbers are not the same) |
| `FileStates[].Favorites[].ScrollPos` | compact struct | | v3.7 | position on the page when the favorite was added (document units; -1 if not stored) |
| `FileStates[].Favorites[].ScrollPos.X` | Float | ` = -1` |  | horizontal position on the page, in document units; -1 if not stored |
| `FileStates[].Favorites[].ScrollPos.Y` | Float | ` = -1` |  | vertical position on the page, in document units; -1 if not stored |
| `FileStates[].Favorites[].IsTemporary` | Bool | ` = false` | internal, v3.7 | session-only favorite; omitted when serializing array elements |
| `FileStates[].EBookUI` | struct | | v3.7 | reflowable (ebook) settings for just this document. The block is absent until you add it; a field left empty or 0 uses the global EBookUI value. The global section's WindowBgCol and DefaultDisplayMode are already per-document as BgCol and DisplayMode below |
| `FileStates[].EBookUI.FontName` | String | ` = ` | v3.7 | font family for this document (e.g. Segoe UI, Microsoft YaHei); empty uses EBookUI.FontName |
| `FileStates[].EBookUI.FontSize` | Float | ` = 0` | v3.7 | font size in points for this document; 0 uses EBookUI.FontSize |
| `FileStates[].EBookUI.Margin` | FloatArray | ` =` | v3.7 | white space around the text for this document, in points; one, two or four values like EBookUI.Margin. empty uses EBookUI.Margin |
| `FileStates[].EBookUI.LineSpacing` | Float | ` = 0` | v3.7 | line-height multiplier for this document (e.g. 1.5); 0 uses EBookUI.LineSpacing |
| `FileStates[].EBookUI.LayoutDx` | Float | ` = 0` | v3.7 | width of the page this document is laid out into, in points; 0 uses EBookUI.LayoutDx |
| `FileStates[].EBookUI.LayoutDy` | Float | ` = 0` | v3.7 | height of the page this document is laid out into, in points; 0 uses EBookUI.LayoutDy |
| `FileStates[].EBookUI.IgnoreDocumentCSS` | String | ` = ` | v3.7 | whether the CSS in this document is ignored: true or false; empty uses EBookUI.IgnoreDocumentCSS |
| `FileStates[].EBookUI.CustomCSS` | String | ` = ` | v3.7 | additional CSS applied to this document; empty uses EBookUI.CustomCSS |
| `FileStates[].TocState` | IntArray | ` =` |  | data required to determine which parts of the table of contents have been expanded |
| `FileStates[].Thumbnail` |  | `NULL` | not saved | thumbnails are saved as PNG files in sumatrapdfcache directory |
| `FileStates[].Himl` |  | `NULL` | not saved | image list holding the file's shell icon |
| `FileStates[].FilePath` | String | ` =` |  | path of the document |
| `FileStates[].DecryptionKey` | String | ` =` |  | data required to open a password protected document without having to ask for the password again |
| `FileStates[].SidebarView` | String | ` =` | v3.7 | what the sidebar's top panel shows: bookmarks (the default), thumbnails or favorites |
| `FileStates[].DisplayMode` | String | ` = automatic` |  | layout of pages. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view |
| `FileStates[].Zoom` | String | ` = fit page` |  | zoom (in %) or one of those values: fit page, fit width, fit height, fit content, fit visible |
| `FileStates[].BgCol` | Color | ` = ` | v3.7 | if given, overrides the background color for this document |
| `FileStates[].PageTextColor` | Color | ` = ` |  | reading foreground color for this file; empty follows the theme |
| `FileStates[].PageBackgroundColor` | Color | ` = ` |  | reading page background for this file; empty follows the theme |
| `FileStates[].TabCol` | Color | ` = ` | v3.7 | if given, overrides the tab color for this document |
| `FileStates[].OpenCount` | Int | ` = 0` |  | number of times this document has been opened recently |
| `FileStates[].PageNo` | String | ` = 1` |  | number of the last read page, or `bm:<bookmark>` for documents with chapters (folds in ReparseIdx; see PagePosition.cpp) |
| `FileStates[].PageCount` | Int | ` = 0` | v3.7 | number of pages in the document when it was last open; 0 if unknown. Used to show reading progress on the home page |
| `FileStates[].Rotation` | Int | ` = 0` |  | how far pages have been rotated as a multiple of 90 degrees |
| `FileStates[].WindowState` | Int | ` = 0` |  | state of the window. 1 is normal, 2 is maximized, 3 is fullscreen, 4 is minimized |
| `FileStates[].SidebarDx` | Int | ` = 0` |  | width of the bookmarks / favorites sidebar in screen pixels, as last resized |
| `FileStates[].Index` | Int | ` = 0` | not saved | temporary value needed for FileHistory::cmpOpenCount |
| `FileStates[].IconIdx` | Int | ` = -1` | not saved | index of the file's shell icon in Himl, -1 if not loaded yet |
| `FileStates[].ScrollPos` | compact struct | |  | how far this document has been scrolled (in x and y direction) |
| `FileStates[].ScrollPos.X` | Float | ` = 0` |  | horizontal scroll offset, in document units |
| `FileStates[].ScrollPos.Y` | Float | ` = 0` |  | vertical scroll offset, in document units |
| `FileStates[].WindowPos` | compact struct | |  | default position (can be on any monitor) |
| `FileStates[].WindowPos.X` | Int | ` = 0` |  | x coordinate of the top-left corner, in screen pixels |
| `FileStates[].WindowPos.Y` | Int | ` = 0` |  | y coordinate of the top-left corner, in screen pixels |
| `FileStates[].WindowPos.Dx` | Int | ` = 0` |  | width, in screen pixels |
| `FileStates[].WindowPos.Dy` | Int | ` = 0` |  | height, in screen pixels |
| `FileStates[].IsPinned` | Bool | ` = false` |  | if true, the document is "pinned" to the Frequently Read list, so that recently opened documents don't displace it |
| `FileStates[].Completed` | Bool | ` = false` |  | show this document in the home page Completed Documents section |
| `FileStates[].IsMissing` | Bool | ` = false` |  | if true, the file is considered missing and won't be shown in any list |
| `FileStates[].UseDefaultState` | Bool | ` = false` |  | if true, this document opens with the global defaults instead of the values below |
| `FileStates[].ShowToc` | Bool | ` = true` |  | if true, show the table of contents (Bookmarks) sidebar when the document has one |
| `FileStates[].DisplayR2L` | Bool | ` = false` |  | if true, the document is displayed right-to-left in facing and book view modes |
| `FileStates[].UniformPageWidth` | Bool | ` = false` | v3.7 | if true, percentage zoom scales every page to the width page 1 has at that zoom level |
| `FileStates[].TrimEmptyMargins` | Bool | ` = false` | v3.7 | if true, empty margins around page content are trimmed from display |
| `FileStates[].FreePan` | Bool | ` = false` | v3.7 | if true, the view can be panned past the page edges, so any part of a page can be brought to the center of the window |
| `SessionData` | array of SessionData | | v3.1 | windows and tabs still open when SumatraPDF was last closed; reopened at startup if RestoreSession is true |
| `SessionData[].TabStates` | array of TabState | |  | data required for restoring the view state of a single tab |
| `SessionData[].TabStates[].FilePath` | String | ` =` |  | path of the document |
| `SessionData[].TabStates[].DisplayMode` | String | ` = automatic` |  | layout of pages in this tab. valid values: automatic, single page, facing, book view, continuous, continuous facing, continuous book view |
| `SessionData[].TabStates[].PageNo` | String | ` = 1` |  | number of the last read page, or `bm:<bookmark>` for documents with chapters (see PagePosition.cpp) |
| `SessionData[].TabStates[].Zoom` | String | ` = fit page` |  | zoom (in %) or one of those values: fit page, fit width, fit height, fit content, fit visible |
| `SessionData[].TabStates[].Rotation` | Int | ` = 0` |  | how far pages have been rotated as a multiple of 90 degrees |
| `SessionData[].TabStates[].ScrollPos` | compact struct | |  | how far this document has been scrolled (in x and y direction) |
| `SessionData[].TabStates[].ScrollPos.X` | Float | ` = 0` |  | horizontal scroll offset, in document units |
| `SessionData[].TabStates[].ScrollPos.Y` | Float | ` = 0` |  | vertical scroll offset, in document units |
| `SessionData[].TabStates[].ShowToc` | Bool | ` = true` |  | if true, the table of contents was shown when the document was closed |
| `SessionData[].TabStates[].SidebarView` | String | ` =` | v3.7 | what the sidebar's top panel showed: bookmarks, thumbnails or favorites |
| `SessionData[].TabStates[].TocState` | IntArray | ` =` |  | which table of contents items were expanded (see FileStates -> TocState) |
| `SessionData[].TabIndex` | Int | ` = 1` |  | index of the currently selected tab (1-based) |
| `SessionData[].WindowState` | Int | ` = 0` |  | state of the window. 1 is normal, 2 is maximized, 3 is fullscreen, 4 is minimized |
| `SessionData[].WindowPos` | compact struct | |  | position of the window (can be on any monitor) |
| `SessionData[].WindowPos.X` | Int | ` = 0` |  | x coordinate of the top-left corner, in screen pixels |
| `SessionData[].WindowPos.Y` | Int | ` = 0` |  | y coordinate of the top-left corner, in screen pixels |
| `SessionData[].WindowPos.Dx` | Int | ` = 0` |  | width, in screen pixels |
| `SessionData[].WindowPos.Dy` | Int | ` = 0` |  | height, in screen pixels |
| `SessionData[].SidebarDx` | Int | ` = 0` |  | width of the favorites / bookmarks sidebar in screen pixels (0 if it wasn't shown) |
| `ReopenOnce` | StringArray | ` =` | v3.0 | data required for reloading documents after an auto-update |
| `TimeOfLastUpdateCheck` | compact struct | |  | data required to determine when SumatraPDF last checked for updates |
| `TimeOfLastUpdateCheck.DwHighDateTime` | Int | ` = 0` |  | high 32 bits of the FILETIME |
| `TimeOfLastUpdateCheck.DwLowDateTime` | Int | ` = 0` |  | low 32 bits of the FILETIME |
| `OpenCountWeek` | Int | ` = 0` |  | value required to determine recency for the OpenCount value in FileStates |
| `LastPrefUpdate` | compact struct | | not saved | modification time of the preferences file when it was last read |
| `LastPrefUpdate.DwHighDateTime` | Int | ` = 0` |  | high 32 bits of the FILETIME |
| `LastPrefUpdate.DwLowDateTime` | Int | ` = 0` |  | low 32 bits of the FILETIME |
| `DefaultDisplayModeEnum` |  | `DM_AUTOMATIC` | not saved | value of DefaultDisplayMode for internal usage |
| `DefaultZoomFloat` | Float | ` = -1` | not saved | value of DefaultZoom for internal usage |
| `PropWinPos` | compact struct | |  | position of the document properties window |
| `PropWinPos.X` | Int | ` = 0` |  | x coordinate, in screen pixels |
| `PropWinPos.Y` | Int | ` = 0` |  | y coordinate, in screen pixels |
| `CheckForUpdates` | Bool | ` = true` | internal | if true, check once a day whether an update is available |

### 6.3 Settings that exist in the committed code but not in `gen-settings.ts`

The committed `src/Settings.h`/`.cpp` has 8 fields that `cmd/gen-settings.ts` does not. Running the generator now would delete them. Treat them as real settings.

| Setting | Type | Default | Description |
|---|---|---|---|
| `FixedPageUI.InvertColorIndex` | Int | `3` | invert colour preset: 0–5 are built-in presets, 6–7 use the custom colours |
| `FixedPageUI.InvertCustomLightCol` | Color | `#fdf6e3` | custom invert preset, light colour |
| `FixedPageUI.InvertCustomDarkCol` | Color | `#181a1f` | custom invert preset, dark colour |
| `DictionaryUI.DictionaryDesign` | Int | `1` | dictionary dialog layout: 0 Classic/Standard, 1 Curated Lexicon, 2 Deep-Work Inspector |
| `DictionaryUI.LearningHubDesign` | Int | `1` | learning hub layout: 0 Classic, 1 Mastery Dashboard, 2 Cognitive Canvas |
| `Annotations.PolygonSides` | Int | `5` | side count of the Polygon tool |
| `Annotations.StarSides` | Int | `5` | point count of the Star tool |
| `Annotations.LassoRectangular` | Bool | `false` | lasso draws a rectangle instead of a freeform path |

---

## 7. Commands and default shortcuts

**How commands work.**

- Every user action is a command, `CmdId`. It can be invoked from the menu, the toolbar, an accelerator, the command palette, `Shortcuts[]` and the `-dbg-control` pipe.
- The numeric IDs come from list order, so refer to commands by name.
- **Shortcut syntax:** `Ctrl+`, `Shift+`, `Alt+` and a key name (`A`, `F3`, `Left`, `Num+`…).
- Single letters such as `J`/`K`/`Q` work only when the canvas has focus.
- **Alternative names:** the names in brackets ("also: …") are extra search terms for the command palette.

Generated from `cmd/gen-commands.ts` and `src/Accelerators.cpp` (378 commands; 88 have default keys). The committed `src/Commands.h` has 12 more, listed in §7.1.

| Command | Name (palette aliases) | Default keys |
|---|---|---|
| `CmdOpenFile` | Open File... | `Ctrl+O` |
| `CmdClose` | Close Document | `Ctrl+W` `Ctrl+F4` |
| `CmdCloseCurrentDocument` | Close Current Document | `Q` |
| `CmdCloseOtherTabs` | Close Other Tabs |  |
| `CmdCloseTabsToTheRight` | Close Tabs To The Right |  |
| `CmdCloseTabsToTheLeft` | Close Tabs To The Left |  |
| `CmdCloseAllTabs` | Close All Tabs |  |
| `CmdSaveAs` | Save File As... | `Ctrl+S` |
| `CmdPrint` | Print Document... | `Ctrl+P` |
| `CmdShowInFolder` | Show File In Folder... |  |
| `CmdRenameFile` | Rename File... | `F2` |
| `CmdDeleteFile` | Delete File |  |
| `CmdExit` | Exit Application | `Ctrl+Q` |
| `CmdReloadDocument` | Reload Document | `R` |
| `CmdCreateShortcutToFile` | Create .lnk Shortcut | `Ctrl+Shift+S` |
| `CmdSendByEmail` | Send Document By Email... |  |
| `CmdProperties` | Document Properties... | `Ctrl+D` |
| `CmdSinglePageView` | Single Page View | `Ctrl+6` `Ctrl+Numpad6` |
| `CmdFacingView` | Facing View | `Ctrl+7` `Ctrl+Numpad7` |
| `CmdBookView` | Book View | `Ctrl+8` `Ctrl+Numpad8` |
| `CmdToggleContinuousView` | Toggle Continuous View | `C` |
| `CmdToggleMangaMode` | Toggle Manga Mode |  |
| `CmdRotateLeft` | Rotate Left | `Ctrl+Shift+Num -` `Ctrl+Shift+-` `[` |
| `CmdRotateRight` | Rotate Right | `Ctrl+Shift+Num +` `Ctrl+Shift++` `]` |
| `CmdToggleBookmarks` | Toggle Bookmarks | `F12` |
| `CmdToggleTableOfContents` | Toggle Table Of Contents |  |
| `CmdToggleFullscreen` | Toggle Fullscreen | `Ctrl+Shift+L` `F11` `F` |
| `CmdPresentationWhiteBackground` | Presentation White Background | `W` |
| `CmdPresentationBlackBackground` | Presentation Black Background | `.` |
| `CmdTogglePresentationMode` | View: Presentation Mode | `Ctrl+L` `F5` `Shift+F11` |
| `CmdToggleToolbar` | Toggle Toolbar | `F8` |
| `CmdChangeScrollbar` | Change Scrollbar... |  |
| `CmdToggleMenuBar` | Toggle Menu Bar | `F9` |
| `CmdCopySelection` | Copy Selection | `Ctrl+C` `Ctrl+Insert` |
| `CmdTranslateSelectionWithGoogle` | Translate Selection with Google |  |
| `CmdTranslateSelectionWithDeepL` | Translate Selection with DeepL |  |
| `CmdSearchSelectionWithGoogle` | Search Selection with Google |  |
| `CmdSearchSelectionWithBing` | Search Selection with Bing |  |
| `CmdSearchSelectionWithWikipedia` | Search Selection with Wikipedia |  |
| `CmdSearchSelectionWithGoogleScholar` | Search Selection with Google Scholar |  |
| `CmdSelectAll` | Select All | `Ctrl+A` |
| `CmdNewWindow` | New Window | `Ctrl+N` |
| `CmdDuplicateInNewWindow` | Open Current Document In New Window | `Ctrl+Shift+N` |
| `CmdDuplicateInNewTab` | Open Current Document In New Tab |  |
| `CmdCopyImage` | Copy Image |  |
| `CmdCopyLinkTarget` | Copy Link Target |  |
| `CmdCopyComment` | Copy Comment |  |
| `CmdCopyFilePath` | Copy File Path |  |
| `CmdScrollUp` | Scroll Up | `K` `Up` |
| `CmdScrollDown` | Scroll Down | `J` `Down` |
| `CmdScrollLeft` | Scroll Left | `H` `Left` |
| `CmdScrollRight` | Scroll Right | `L` `Right` |
| `CmdScrollLeftPage` | Scroll Left By Page | `Shift+Left` |
| `CmdScrollRightPage` | Scroll Right By Page | `Shift+Right` |
| `CmdScrollUpPage` | Scroll Up By Page | `PgUp` `Shift+Space` `Shift+Enter` `Ctrl+Up` |
| `CmdScrollDownPage` | Scroll Down By Page | `PgDn` `Space` `Enter` `Ctrl+Down` |
| `CmdScrollDownHalfPage` | Scroll Down By Half Page | `Shift+Down` |
| `CmdScrollUpHalfPage` | Scroll Up By Half Page | `Shift+Up` |
| `CmdGoToNextPage` | Next Page | `N` `Ctrl+PgDn` |
| `CmdGoToPrevPage` | Previous Page | `P` `Ctrl+PgUp` |
| `CmdGoToFirstPage` | First Page | `Home` `Ctrl+Home` |
| `CmdGoToLastPage` | Last Page | `End` `Ctrl+End` |
| `CmdGoToPage` | Go to Page... | `Ctrl+G` `G` |
| `CmdFindFirst` | Find... | `Ctrl+F` |
| `CmdFindNext` | Find Next | `F3` |
| `CmdFindPrev` | Find Previous | `Shift+F3` |
| `CmdFindNextSel` | Find Next Selection | `Ctrl+F3` |
| `CmdFindPrevSel` | Find Previous Selection | `Ctrl+Shift+F3` |
| `CmdFindToggleMatchCase` | Find: Toggle Match Case |  |
| `CmdSaveAnnotations` | Save Annotations to existing PDF | `Ctrl+Shift+S` |
| `CmdSaveAnnotationsNewFile` | Save Annotations to a new PDF... |  |
| `CmdDiscardChanges` | Discard Changes |  |
| `CmdDeleteAnnotation` | Delete Annotation | `Ctrl+Delete` |
| `CmdZoomFitPage` | Zoom: Fit Page | `Ctrl+0` `Ctrl+Numpad0` |
| `CmdZoomActualSize` | Zoom: Actual Size | `Ctrl+1` `Ctrl+Numpad1` |
| `CmdZoomFitWidth` | Zoom: Fit Width | `Ctrl+2` `Ctrl+Numpad2` |
| `CmdZoomFitByOrientation` | Zoom: Fit Page or Width by Orientation |  |
| `CmdZoom6400` | Zoom: 6400% |  |
| `CmdZoom3200` | Zoom: 3200% |  |
| `CmdZoom1600` | Zoom: 1600% |  |
| `CmdZoom800` | Zoom: 800% |  |
| `CmdZoom400` | Zoom: 400% |  |
| `CmdZoom200` | Zoom: 200% |  |
| `CmdZoom150` | Zoom: 150% |  |
| `CmdZoom125` | Zoom: 125% |  |
| `CmdZoom100` | Zoom: 100% |  |
| `CmdZoom50` | Zoom: 50% |  |
| `CmdZoom25` | Zoom: 25% |  |
| `CmdZoom12_5` | Zoom: 12.5% |  |
| `CmdZoom8_33` | Zoom: 8.33% |  |
| `CmdZoomFitContent` | Zoom: Fit Content | `Ctrl+3` `Ctrl+Numpad3` |
| `CmdZoomShrinkToFit` | Zoom: Shrink To Fit |  |
| `CmdZoomCustom` | Zoom: Custom... | `Ctrl+Y` |
| `CmdZoomIn` | Zoom In | `Ctrl+Num +` `Ctrl++` |
| `CmdZoomOut` | Zoom Out | `Ctrl+Num -` `Ctrl+-` |
| `CmdZoomFitWidthAndContinuous` | Zoom: Fit Width And Continuous |  |
| `CmdZoomFitPageAndSinglePage` | Zoom: Fit Page and Single Page |  |
| `CmdContributeTranslation` | Contribute Translation |  |
| `CmdOpenWithExplorer` | Open Directory In Explorer |  |
| `CmdOpenWithDirectoryOpus` | Open Directory In Directory Opus |  |
| `CmdOpenWithTotalCommander` | Open Directory In Total Commander |  |
| `CmdOpenWithDoubleCommander` | Open Directory In Double Commander |  |
| `CmdOpenWithAcrobat` | Open in Adobe Acrobat |  |
| `CmdOpenWithFoxit` | Open in Foxit Reader |  |
| `CmdOpenWithFoxitPhantom` | Open in Foxit PhantomPDF |  |
| `CmdOpenWithPdfXchange` | Open in PDF-XChange |  |
| `CmdOpenWithXpsViewer` | Open in Microsoft XPS Viewer |  |
| `CmdOpenWithHtmlHelp` | Open in Microsoft HTML Help |  |
| `CmdOpenWithPdfDjvuBookmarker` | Open With Pdf&Djvu Bookmarker |  |
| `CmdOpenSelectedDocument` | Open Selected Document |  |
| `CmdPinSelectedDocument` | Pin Selected Document |  |
| `CmdForgetSelectedDocument` | Remove Selected Document From History |  |
| `CmdExpandAll` | Expand All |  |
| `CmdCollapseAll` | Collapse All |  |
| `CmdSaveEmbeddedFile` | Save Embedded File... |  |
| `CmdOpenEmbeddedPDF` | Open Embedded PDF |  |
| `CmdSaveAttachment` | Save Attachment... |  |
| `CmdOpenAttachment` | Open Attachment |  |
| `CmdOptions` | Settings... |  |
| `CmdAdvancedSettings` | Advanced Settings... (also: Advanced Options...) |  |
| `CmdChangeLanguage` | Change Language... |  |
| `CmdCheckUpdate` | Check For Updates |  |
| `CmdInstallPrereleaseUpdate` | Install Pre-release Update |  |
| `CmdTogglePdfPreviewLogging` | Toggle PDF Preview Logging |  |
| `CmdHelpOpenManual` | Help: Manual | `F1` |
| `CmdHelpOpenManualOnWebsite` | Help: Manual On Website |  |
| `CmdHelpOpenKeyboardShortcuts` | Help: Keyboard Shortcuts |  |
| `CmdToggleKeyboardHelp` | Show Keyboard Shortcuts | `Shift+/` |
| `CmdHelpVisitWebsite` | Help: SumatraPDF Website |  |
| `CmdHelpAbout` | Help: About SumatraPDF... |  |
| `CmdMoveFrameFocus` | Move Frame Focus | `F6` |
| `CmdFavoriteAdd` | Add Favorite |  |
| `CmdFavoriteDel` | Delete Favorite |  |
| `CmdFavoriteToggle` | Toggle Favorites |  |
| `CmdToggleLinks` | Toggle Show Links |  |
| `CmdToggleShowAnnotations` | Toggle Show Annotations |  |
| `CmdShowAnnotations` | Show Annotations |  |
| `CmdHideAnnotations` | Hide Annotations |  |
| `CmdCreateAnnotText` | Create Text Annotation |  |
| `CmdCreateAnnotLink` | Create Link Annotation |  |
| `CmdCreateAnnotFreeText` | Create Free Text Annotation |  |
| `CmdCreateAnnotLine` | Create Line Annotation |  |
| `CmdCreateAnnotSquare` | Create Square Annotation |  |
| `CmdCreateAnnotCircle` | Create Circle Annotation |  |
| `CmdCreateAnnotPolygon` | Create Polygon Annotation |  |
| `CmdCreateAnnotPolyLine` | Create Polyline Annotation |  |
| `CmdCreateAnnotHighlight` | Create Highlight Annotation | `A` `Shift+A` |
| `CmdCreateAnnotUnderline` | Create Underline Annotation | `U` `Shift+U` |
| `CmdCreateAnnotSquiggly` | Create Squiggly Annotation |  |
| `CmdCreateAnnotStrikeOut` | Create Strike Out Annotation |  |
| `CmdCreateAnnotRedact` | Create Redact Annotation |  |
| `CmdCreateAnnotStamp` | Create Stamp Annotation |  |
| `CmdCreateAnnotCaret` | Create Caret Annotation |  |
| `CmdCreateAnnotInk` | Create Ink Annotation |  |
| `CmdCreateAnnotPopup` | Create Popup Annotation |  |
| `CmdCreateAnnotFileAttachment` | Create File Attachment Annotation |  |
| `CmdInvertColors` | Invert Colors | `Shift+I` |
| `CmdTogglePageInfo` | Toggle Page Info | `I` |
| `CmdToggleZoom` | Toggle Zoom | `Z` |
| `CmdNavigateBack` | Navigate Back | `Backspace` `Alt+Left` |
| `CmdNavigateForward` | Navigate Forward | `Shift+Backspace` `Alt+Right` |
| `CmdToggleCursorPosition` | Toggle Cursor Position | `M` |
| `CmdOpenNextFileInFolder` | Open Next File In Folder | `Ctrl+Shift+Right` |
| `CmdOpenPrevFileInFolder` | Open Previous File In Folder | `Ctrl+Shift+Left` |
| `CmdCommandPalette` | Command Palette | `Ctrl+K` |
| `CmdShowLog` | Show Logs |  |
| `CmdShowErrors` | Show Errors... |  |
| `CmdClearHistory` | Clear History |  |
| `CmdReopenLastClosedFile` | Reopen Last Closed | `Ctrl+Shift+T` |
| `CmdNextTab` | Next Tab | `Ctrl+PgDn` |
| `CmdPrevTab` | Previous Tab | `Ctrl+PgUp` |
| `CmdNextTabSmart` | Smart Next Tab | `Ctrl+Tab` |
| `CmdPrevTabSmart` | Smart Previous Tab | `Ctrl+Shift+Tab` |
| `CmdMoveTabLeft` | Move Tab Left | `Ctrl+Shift+PgUp` |
| `CmdMoveTabRight` | Move Tab Right | `Ctrl+Shift+PgDn` |
| `CmdInvokeInverseSearch` | Invoke Inverse Search |  |
| `CmdExec` | Execute a program |  |
| `CmdViewWithExternalViewer` | View With Custom External Viewer |  |
| `CmdSelectionHandler` | Launch a browser or run command with selection |  |
| `CmdSetTheme` | Set theme |  |
| `CmdToggleInverseSearch` | Toggle Inverse Search |  |
| `CmdDebugCorruptMemory` | Debug: Corrupt Memory |  |
| `CmdDebugCrashMe` | Debug: Crash Me |  |
| `CmdDebugShowNotif` | Debug: Show Notification |  |
| `CmdDebugStartStressTest` | Debug: Start Stress Test |  |
| `CmdDebugTogglePredictiveRender` | Debug: Toggle Predictive Rendering |  |
| `CmdDebugToggleRtl` | Debug: Toggle RTL |  |
| `CmdListPrinters` | List Printers... |  |
| `CmdToggleWindowsPreviewer` | Toggle Windows Previewer |  |
| `CmdToggleWindowsSearchFilter` | Toggle Windows Search Filter |  |
| `CmdScreenshot` | Take Screenshot... |  |
| `CmdCropImage` | Crop Image... |  |
| `CmdResizeImage` | Resize Image... |  |
| `CmdSaveImage` | Save Image... |  |
| `CmdPasteClipboardImage` | Paste Image From Clipboard | `Ctrl+V` |
| `CmdTabGroupSave` | Save Tab Group... |  |
| `CmdTabGroupRestore` | Restore Tab Group... |  |
| `CmdChangeBackgroundColor` | Change Background Color... |  |
| `CmdChangeEbookSettings` | Change eBook Settings... |  |
| `CmdSetTabColor` | Change Tab Color... |  |
| `CmdPdfCompress` | Compress PDF... |  |
| `CmdPdfDecompress` | Decompress PDF... |  |
| `CmdPdfDeletePages` | Delete Pages From PDF... |  |
| `CmdPdfExtractPages` | Extract Pages From PDF... |  |
| `CmdPdfEncrypt` | Encrypt PDF... |  |
| `CmdPdfDecrypt` | Decrypt PDF... |  |
| `CmdPdfBake` | Bake PDF File... |  |
| `CmdPdfShowInfo` | Show PDF Info... |  |
| `CmdDocumentExtractText` | Extract Text From Document... |  |
| `CmdDocumentShowOutline` | Show Document Bookmarks... |  |
| `CmdSetScreenshotHotkey` | Set Screenshot Hotkey... |  |
| `CmdToggleReadAloud` | Toggle Read Aloud |  |
| `CmdPauseReadAloud` | Pause Reading |  |
| `CmdContinueReadAloud` | Continue Reading |  |
| `CmdStopReadAloud` | Stop Reading |  |
| `CmdReadAloudFromTopPage` | Start Reading From Top |  |
| `CmdReadAloudSelection` | Start Reading Selection |  |
| `CmdToggleToolbarShowReadAloud` | Read Aloud: Show In Toolbar |  |
| `CmdRemoveDeletedFilesFromHistory` | Remove Deleted Files From History |  |
| `CmdCommandPaletteTOC` | Command Palette: Table Of Contents | `Shift+F12` |
| `CmdDebugToggleRenderInfo` | Debug: Toggle Render Queue Info |  |
| `CmdConvertImageToPdf` | Convert Image To PDF... |  |
| `CmdExpandToCurrentPage` | Expand TOC to Current Page |  |
| `CmdStartAutoScroll` | Start Auto-Scroll |  |
| `CmdAIChatWithClaudeCode` | Claude chat... |  |
| `CmdAIChatWithGrokBuild` | Grok chat... |  |
| `CmdAIChatWithOpenAICodex` | Codex chat... |  |
| `CmdTranslateSelectionWithGrokBuild` | Translate Selection with Grok Build... |  |
| `CmdTranslateSelectionWithClaudeCode` | Translate Selection with Claude Code... |  |
| `CmdTranslateSelectionWithOpenAICodex` | Translate Selection with OpenAI Codex... |  |
| `CmdFindToggleMatchWholeWord` | Find: Toggle Match Whole Word |  |
| `CmdGoToNextFavorite` | Go to Next Favorite |  |
| `CmdGoToPrevFavorite` | Go to Previous Favorite |  |
| `CmdCreateAnnotImageFromClipboard` | Create Image Annotation From Clipboard |  |
| `CmdSetInverseSearch` | Set Inverse Search Command Line... |  |
| `CmdCommandPaletteFavorites` | Command Palette: Favorites |  |
| `CmdNavigateFilesInFolder` | Navigate Files in Folder... (also: Browse Files In Folder...) | `Ctrl+Shift+Up` |
| `CmdDebugToggleCacheInfo` | Debug: Toggle Cache Info |  |
| `CmdToggleEngineeringDrawingEnhance` | Toggle Engineering Drawing Enhancement |  |
| `CmdSetDocumentColorsFollowTheme` | Make Document Colors Follow Theme... |  |
| `CmdTogglePreservePdfImages` | Toggle Preserve PDF Image Colors in Dark Mode |  |
| `CmdToggleLightDarkTheme` | Toggle Light/Dark Theme |  |
| `CmdChangeTheme` | Change Theme... |  |
| `CmdTranslateSelection` | Translate Selection... |  |
| `CmdFavoriteShowInTab` | Show Favorites in Tab |  |
| `CmdTocExpandToLevel1` | Bookmarks: Expand to Level 1 |  |
| `CmdTocExpandToLevel2` | Bookmarks: Expand to Level 2 |  |
| `CmdTocExpandToLevel3` | Bookmarks: Expand to Level 3 |  |
| `CmdTocCollapseSameLevel` | Bookmarks: Collapse Same Level |  |
| `CmdToggleFavoritesSort` | Sort Favorites By Name |  |
| `CmdZoomFitHeight` | Zoom: Fit Height |  |
| `CmdDeleteFileAndOpenNext` | Delete File And Open Next |  |
| `CmdShowGeneratedHTML` | Show Generated HTML |  |
| `CmdDeleteCachedFiles` | Delete Cached Files |  |
| `CmdToggleKeyboardLinkFollowing` | Follow Link With Keyboard | `Shift+F` |
| `CmdDebugToggleDpiOverride` | Debug: Toggle DPI Override |  |
| `CmdToggleImages` | Toggle Show Images |  |
| `CmdSelectTextViaKeyboard` | Select Text With Keyboard | `F7` |
| `CmdOpenFileWithOSFilePicker` | Open File With Windows File Picker... |  |
| `CmdToggleFilePicker` | SumatraPDF File Picker |  |
| `CmdToggleBoolSetting` | Toggle Boolean Setting |  |
| `CmdFixDefaultApp` | Fix Default App For Extension |  |
| `CmdAIChatWithAntiGravity` | Antigravity chat... |  |
| `CmdTranslateSelectionWithAntiGravity` | Translate Selection with Antigravity... |  |
| `CmdConvertToPDF` | Convert To PDF... |  |
| `CmdDebugShowFitContentArea` | Debug: Show Fit Content Area |  |
| `CmdExtendSelectionCharLeft` | Extend Selection One Character Left |  |
| `CmdExtendSelectionCharRight` | Extend Selection One Character Right |  |
| `CmdExtendSelectionWordLeft` | Extend Selection One Word Left |  |
| `CmdExtendSelectionWordRight` | Extend Selection One Word Right |  |
| `CmdToggleLaserPointer` | Toggle Laser Pointer |  |
| `CmdZoomToSelection` | Zoom: To Selection | `Ctrl+4` `Ctrl+Numpad4` |
| `CmdToggleHoverPreview` | Toggle Citation Hover Preview |  |
| `CmdToggleDisableLinks` | Toggle Disable Links |  |
| `CmdSignDocument` | Sign Document... |  |
| `CmdInsertImage` | Insert Image... |  |
| `CmdToggleHighlightFormFields` | Toggle Highlight Form Fields |  |
| `CmdTogglePageBoxes` | Toggle Page Boxes |  |
| `CmdConvertPdfToImages` | Convert PDF to Images... |  |
| `CmdToggleUniformPageWidth` | Toggle Uniform Page Width |  |
| `CmdToggleTransparencyGrid` | Toggle Transparency Grid |  |
| `CmdTogglePageGrid` | Toggle Page Grid |  |
| `CmdConfigurePageGrid` | Configure Page Grid... |  |
| `CmdToggleEditPDF` | Toggle Edit PDF |  |
| `CmdApplyRedactions` | Apply Redactions |  |
| `CmdUndo` | Undo | `Ctrl+Z` |
| `CmdRedo` | Redo | `Ctrl+Shift+Z` |
| `CmdCutAnnotation` | Cut Annotation | `Ctrl+X` |
| `CmdCopyAnnotation` | Copy Annotation |  |
| `CmdPasteAnnotation` | Paste Annotation |  |
| `CmdSearchGoogleLens` | Search with Google Lens |  |
| `CmdNavigateThumbnail` | Navigate Thumbnails |  |
| `CmdShowAnnotationText` | Show Comment |  |
| `CmdAnnotationHighlightBrush` | Highlighter |  |
| `CmdFindAnnotation` | Find Annotation |  |
| `CmdOpenFileNoHistory` | Open File Without History... |  |
| `CmdCopySelectionAsImage` | Copy Selection As Image |  |
| `CmdSearchGoogleLensPage` | Search Page with Google Lens |  |
| `CmdSearchGoogleLensImage` | Search Image with Google Lens |  |
| `CmdSaveSelectionAsImage` | Save As Image... |  |
| `CmdToggleTrimEmptyMargins` | Toggle Trim Empty Margins |  |
| `CmdCopyLocationToClipboard` | Copy Location To Clipboard |  |
| `CmdToggleAutomaticallyScroll` | Automatically Scroll | `Ctrl+Shift+H` |
| `CmdAutomaticallyScrollFaster` | Automatically Scroll Faster |  |
| `CmdAutomaticallyScrollSlower` | Automatically Scroll Slower |  |
| `CmdToggleReadingBar` | Reading Bar |  |
| `CmdToggleReadingBarInvert` | Reading Bar Invert |  |
| `CmdGoToHomePage` | Go To Home Page |  |
| `CmdToggleFreePan` | Toggle Free Pan |  |
| `CmdNone` | Do nothing |  |
| `CmdFileHistory` | Open Recent File |  |
| `CmdFavorite` | Go to Favorite |  |
| `CmdReadAloudFromCursorPosition` | Start Reading From Cursor Position |  |
| `CmdToggleGrayscale` | Toggle Grayscale |  |
| `CmdPrintSelection` | Print Selection... |  |
| `CmdAutoGenerateTOC` | Generate Table Of Contents |  |
| `CmdOpenSettingsFile` | Open Advanced Settings File... |  |
| `CmdOpenFileWithSumatraFilePicker` | Open File With SumatraPDF File Picker... |  |
| `CmdSelectCurrentPage` | Select Current Page |  |
| `CmdZoomFitVisible` | Zoom: Fit Visible |  |
| `CmdSignWithImage` | Sign With Image |  |
| `CmdInsertTextSnippet` | Insert Text Snippet |  |
| `CmdInkPen` | Pen: Write |  |
| `CmdInkHighlighter` | Pen: Freehand Highlighter |  |
| `CmdInkEraser` | Pen: Stroke Eraser |  |
| `CmdHighlightEraser` | Pen: Highlighter-only Eraser |  |
| `CmdInkBlack` | Pen Color: Black |  |
| `CmdInkBlue` | Pen Color: Blue |  |
| `CmdInkRed` | Pen Color: Red |  |
| `CmdInkThin` | Pen Width: Thin |  |
| `CmdInkMedium` | Pen Width: Medium |  |
| `CmdInkThick` | Pen Width: Thick |  |
| `CmdTogglePenOnly` | Pen: Toggle Palm Rejection |  |
| `CmdThemeLight` | Theme: Light |  |
| `CmdThemeDark` | Theme: Dark |  |
| `CmdInkFountain` | Pen: Fountain |  |
| `CmdInkBrush` | Pen: Brush |  |
| `CmdInkPencil` | Pen: Pencil |  |
| `CmdLaserSolid` | Laser Pointer: Solid Trail |  |
| `CmdLaserHollow` | Laser Pointer: Hollow Trail |  |
| `CmdLaserDot` | Laser Pointer: Single Dot |  |
| `CmdDictionaryLookup` | Dictionary: Define Selected Word | `Shift+D` |
| `CmdVocabularyHome` | Vocabulary: Learn and Practice |  |
| `CmdExportStudyNotes` | Export Highlights and Notes... |  |
| `CmdEditBookmarks` | Edit PDF Bookmarks... |  |
| `CmdAnnotationLasso` | Lasso annotation selection |  |
| `CmdHandTool` | Hand tool: drag to pan |  |
| `CmdToggleThumbnails` | Toggle Thumbnails |  |
| `CmdMergePDF` | Merge PDF... |  |
| `CmdInkSegmentEraser` | Eraser: Remove Ink Segments |  |
| `CmdLassoRotateLeft` | Selected Annotations: Rotate Left |  |
| `CmdLassoRotateRight` | Selected Annotations: Rotate Right |  |
| `CmdLassoDuplicate` | Selected Annotations: Duplicate |  |
| `CmdLassoRecolor` | Selected Annotations: Apply Pen Color |  |
| `CmdLassoThinner` | Selected Annotations: Thinner |  |
| `CmdLassoThicker` | Selected Annotations: Thicker |  |
| `CmdTextSelectTool` | Select text: drag or Shift-click |  |
| `CmdLassoFreehand` | Lasso: Freehand Selection |  |
| `CmdLassoRectangle` | Lasso: Rectangular Selection |  |
| `CmdLassoDelete` | Lasso: Delete Selected Annotations |  |
| `CmdExportAnnotationsTransfer` | Export Annotations for Transfer... |  |
| `CmdImportAnnotationsTransfer` | Import Annotations from Another Version... |  |
| `CmdAnnotationShapes` | Shapes |  |
| `CmdAnnotationMode` | Annotation Mode |  |
| `CmdExportSettings` | Export Settings... |  |
| `CmdImportSettings` | Import Settings... |  |
| `CmdBackupSettings` | Back Up Settings Now... |  |
| `CmdCreateAnnotArrow` | Create Arrow Annotation |  |
| `CmdCreateAnnotDashedLine` | Create Dashed Line Annotation |  |
| `CmdEditMarkdown` | Edit Markdown... |  |
| `CmdCreateAnnotParabola` | Create Parabola Annotation |  |
| `CmdCreateAnnotNegativeParabola` | Create Negative Parabola Annotation |  |
| `CmdCreateAnnotCubic` | Create Cubic Annotation |  |
| `CmdCreateAnnotNegativeCubic` | Create Negative Cubic Annotation |  |
| `CmdCreateAnnotReciprocal` | Create Reciprocal Annotation |  |
| `CmdCreateAnnotNegativeReciprocal` | Create Negative Reciprocal Annotation |  |
| `CmdCreateAnnotAxes2D` | Create 2D Axes Annotation |  |
| `CmdCreateAnnotAxes3D` | Create 3D Axes Annotation |  |

### 7.1 Commands in the committed code but not in `gen-commands.ts`

`src/Commands.h` defines 12 commands that `cmd/gen-commands.ts` does not list (the `CmdFirst`/`CmdLast` range markers are left out). Running the generator now would delete them.

| Command | Name | Default keys |
|---|---|---|
| `CmdAddBookmark` | Add Bookmark (Current Page): adds a PDF bookmark when the PDF can be edited, otherwise a favorite | `Ctrl+B` |
| `CmdRenameBookmark` | Rename Bookmark (the selected TOC entry) |  |
| `CmdDeleteBookmark` | Delete Bookmark (the selected TOC entry) |  |
| `CmdCreateAnnotStar` | Create Star Annotation (`Annotations.StarSides` points) |  |
| `CmdInvertColorPureWhite` | Invert colours preset: Pure White (Light) |  |
| `CmdInvertColorWarmSepia` | Invert colours preset: Warm Sepia (Light) |  |
| `CmdInvertColorSoftGray` | Invert colours preset: Soft Paper Gray (Light) |  |
| `CmdInvertColorPureBlack` | Invert colours preset: Pure Black (Dark) |  |
| `CmdInvertColorCharcoal` | Invert colours preset: Midnight Charcoal (Dark) |  |
| `CmdInvertColorWarmEspresso` | Invert colours preset: Warm Espresso (Dark) |  |
| `CmdInvertColorCustomLight` | Custom Light Background… (`InvertCustomLightCol`) |  |
| `CmdInvertColorCustomDark` | Custom Dark Background… (`InvertCustomDarkCol`) |  |

---

## 8. Command-line flags

Defined in `cmd/gen-flags.ts`, parsed in `src/Flags.cpp`. A flag is written `-name` (also `/name`). Arguments follow it, separated by a space. Any argument that is not a flag is a file to open.

### 8.1 Opening and view

| Flag | Argument | Effect |
|---|---|---|
| `-page` | n | open at page n |
| `-named-dest`, `-nameddest` | name | open at a named destination |
| `-view` | mode | display mode, e.g. `"continuous facing"` |
| `-zoom` | z | `fit page`, `fit width`, `fit content` or a percentage |
| `-scroll` | x,y | scroll offset on the page |
| `-search` | term | start a search for the term |
| `-presentation`, `-fullscreen` | | start in that mode |
| `-invertcolors`, `-invert-colors` | | invert document colours |
| `-manga-mode` | true/false | right-to-left comics |
| `-bgcolor`, `-bg-color` | colour | window background |
| `-set-color-range` | text bg | text and background colour range |
| `-window-pos` | x y w h | initial window rectangle |
| `-new-window` | | open in a new window, not a new tab |
| `-new-window-tabs` | | open each file in its own window |
| `-reuse-instance` | | send to the running instance |
| `-esc-to-exit` | | Escape closes the app |
| `-restrict` | | restricted mode (§10) |
| `-lang` | code | UI language |
| `-appdata` | dir | settings and data folder |
| `-plugin` | url hwnd | embed in a host window (browser plugin mode) |
| `-pwd` | password | document password |
| `-html-backend` | name | engine choice for HTML/EPUB |
| `-console` | | attach a console for output |
| `-a` | | (internal) |
| `-dde` | cmd | run DDE commands on the running instance |

### 8.2 Printing

| Flag | Argument | Effect |
|---|---|---|
| `-print-to-default` | | print to the default printer and exit |
| `-print-to` | printer | print to the named printer |
| `-print-dialog` | | show the print dialog |
| `-p` | | print dialog (the form Adobe Reader uses) |
| `-t` | printer | silent print (the form Adobe Reader uses) |
| `-print-settings` | list | comma-separated tokens: page ranges, `odd`, `even`, `noscale`, `shrink`, `fit`, `stretch`, `center`, `portrait`, `landscape`, `color`, `monochrome`, `duplex`, `duplexshort`, `duplexlong`, `simplex`, `bin=…`, `paper=…`, `<n>x` |
| `-exit-on-print`, `-exit-when-done` | | exit after printing or the task |
| `-enum-printers`, `-list-printers` | | list printers |

### 8.3 LaTeX (SyncTeX)

| Flag | Argument | Effect |
|---|---|---|
| `-inverse-search` | cmdline | inverse-search command (`%f`, `%l`, `%c`) |
| `-forward-search`, `-fwdsearch` | srcfile line | forward search |
| `-fwdsearch-offset`, `-fwdsearch-width`, `-fwdsearch-color`, `-fwdsearch-permanent` | value | look of the forward-search highlight |

### 8.4 Installer and updates

| Flag | Argument | Effect |
|---|---|---|
| `-install`, `-uninstall` | | installer or uninstaller mode |
| `-s`, `-silent`, `-fast-install` | | no UI |
| `-d`, `-install-dir` | dir | install location |
| `-all-users`, `-allusers` | | install for all users |
| `-with-filter`, `-with-search`, `-with-preview` | | register the search filter or preview handler |
| `-no-desktop-shortcut` | | skip the shortcut |
| `-run-install-now`, `-update-self-to`, `-upgrade-from`, `-delete-file` | | update hand-off steps |
| `-x` | | extract files |

### 8.5 Utilities

| Flag | Argument | Effect |
|---|---|---|
| `-quicklook` | file | Explorer QuickLook preview window |
| `-quicklook-agent` | | background agent |
| `-dump-exif` | file | print EXIF data |
| `-dump-chm` | file | dump the contents of a CHM file |
| `-extract-text` | file | print the text |
| `-render` | file | render pages (benchmark) |
| `-bench` | file [pages] | rendering benchmark |
| `-engine-dump` | file | dump the engine's view of the document |

### 8.6 Testing and diagnostics

| Flag | Argument | Effect |
|---|---|---|
| `-for-testing` | | new instance; no session restore; no settings save |
| `-for-ai` | | non-interactive diagnostics for agents |
| `-unit-tests` | | run built-in unit tests (debug builds) |
| `-dbg-control` | pipe | binary control pipe (§9.2) |
| `-log`, `-log-to-file` | [path] | logging |
| `-start-perf-log`, `-log-perf-file` | [path] | performance log |
| `-stress-test` | path | stress test (`-n`, `-max`, `-max-files`, `-rand`) |
| `-regress`, `-tester`, `-testapp`, `-test-plugin`, `-test-preview` | | internal test modes |
| `-crash-on-open`, `-sleep-ms` | | fault injection |

---

## 9. Automation interfaces

### 9.1 DDE

Service and topic: `SUMATRA` / `control`. A command is a string of one or more `[Cmd(args)]` items. Strings are double-quoted, and booleans are 0 or 1.

| Command | Effect |
|---|---|
| `[Open("file"[,newWindow,setFocus,forceRefresh])]` | open a file |
| `[ForwardSearch(["pdf",]"src",line,col[,newWindow,setFocus])]` | SyncTeX forward search. The PDF can be left out when it is already open, so editors do not need to know about multi-file projects. |
| `[GotoPage("file",page)]` | jump to a page |
| `[GotoPageWord("file",page,"word")]` | jump to a page and highlight a word |
| `[Search("file","term")]` | search |
| `[NewWindow]` | open a new window |
| `[GetFileState("file")]` | return the page, zoom and view |
| `[GetOpenFiles]` | list the open files |
| `[GetMousePos]` | return the cursor position on the page |
| `[SetView("file","mode",zoom[,scrollX,scrollY])]` | set the view mode and zoom. Zoom is a percentage, or −1 fit page, −2 fit width, −3 fit content. |
| `[CmdName]` (e.g. `[CmdNextPage]`) | run any command by name in the window (`HandleCmdCommand`) |

### 9.2 `-dbg-control` named pipe (tests and agents)

**Request:** `[u32 payloadSize][u16 command][u16 requestId][args…]`.
**Response:** `[u32 payloadSize][u16 requestId][results…]`.

Each argument or result is `[u16 type]` followed by its data:

| Type | Meaning | Data |
|---|---|---|
| 0 | end | none |
| 1 | i32 | 4 bytes |
| 2 | bytes | u32 length, then the data |
| 3 | UTF-8 string | u32 length, the bytes, then a NUL |
| 4 | list | u16 count, then the encoded elements |

**Uses:** sending commands, reading state, taking screenshots, turning notifications off, and quitting. See `tests/control.ts`.

---

## 10. Restricted mode (`sumatrapdfrestrict.ini`)

**How it works.**

- Restricted mode starts when a `sumatrapdfrestrict.ini` file sits next to the exe, or with `-restrict`.
- Section `[Policies]`. Each value is 0 or 1; a missing value counts as 0.
- In restricted mode, settings transfer is disabled.

| Policy | Controls |
|---|---|
| `InternetAccess` | update checks and crash reports. Enhanced also checks it for the cpdf download, dictionary downloads, translation, Google Lens and Markdown web fonts. |
| `DiskAccess` | file dialogs, saving, opening links in a browser, external viewers and editors, Frequently Read, reopening recent files |
| `SavePreferences` | saving settings, favorites and recent files |
| `RegistryAccess` | setting the default viewer |
| `PrinterAccess` | printing |
| `CopySelection` | selecting and copying |
| `FullscreenAccess` | fullscreen and presentation |
| `LinkProtocols` | link protocols handed to the OS (default `http,https,mailto`) |
| `SafeFileTypes` | PerceivedTypes opened externally (default `audio,video,webpage`; `*` = all) |

---

## 11. Guidance for a rewrite

**Port in this order.** Each step is usable on its own.

1. Engine abstraction:
   - page model and coordinates;
   - render to bitmap;
   - text with glyph boxes;
   - links and TOC.
2. Layout and viewport: display modes and zoom (`DisplayModel`).
3. Tiled render cache with background workers.
4. Settings tree with the sparse text format and per-file state.
5. Command table with accelerators, the menu, the toolbar and the palette.
6. Tabs, windows and sessions.
7. Search and selection.
8. Annotations, using MuPDF's journal for undo.
9. Then the Enhanced features in any order: they sit on top of steps 1–8.

**Invariants to keep.**

- Annotation geometry is stored in page space, one annotation per page. PDF ink holds one width per stroke.
- Every background result must check, before it is applied, that its tab still exists and still holds the same document.
- Writes to the user's files are atomic (temp file, then rename). Tools save copies; they never replace an existing file unless the user says so.
- Every download is verified: HTTPS only, no redirects, expected size and hash where known, and bounded reads.
- Selected text never leaves the machine except through explicit online commands (translate, Lens, online dictionary, AI chat).
- Settings: write only values that differ from the default. Ignore unknown keys, so old and new versions can share one file.
- Command IDs are internal; persist and expose command names.

**Built on MuPDF.**

- MuPDF features used: the PDF journal (undo), annotation creation and appearance, widgets and JavaScript (forms), `pdf_sign_signature`, the outline API, `fz_device` wrappers (dark mode, CAD), text extraction (`fz_stext`), and incremental save.
- `ext/patches/` holds 36 Sumatra patches to MuPDF. Re-apply them, or reimplement them, in any port that uses MuPDF.

**Tests to port first.**

- Pointer-sample normalisation and lasso geometry: `Canvas_UnitTest*`.
- Reference-hover detectors: `src/base/tests/RefHover_ut.cpp`.
- Dark-mode OKLab and image classifier: `PdfDarkMode*_ut.cpp`.
- Vocabulary scheduling: `Vocabulary.cpp` unit tests.
- Kaikki index: u64 offsets and hash-collision filtering.
- Dictionary parsing.
- Issue regression scripts: `tests/issue-*.ts`, which drive the app through `-dbg-control`.

**Not shipped yet (roadmap).**

- Ink: per-point pressure and tilt rendering, textured nibs, saving pressure and tilt, strokes across pages, handwriting cleanup, shape recognition.
- OCR.
- Cross-page lasso.
- Zoom choices from 25% to 1200% in the picker (READ-003).
- See `SumatraPDF_Enhanced_TODO.md`.
