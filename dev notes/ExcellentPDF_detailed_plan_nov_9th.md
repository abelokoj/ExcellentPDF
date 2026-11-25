# ExcellentPDF — combined detailed plan

**Date:** 9 November 2026
**Status:** Working plan; supersedes the stack choices in the five source documents (see §19)
**Stack (decided):** Rust; Native Shell (NSH, <https://nsh.dev>); MuPDF, the engine SumatraPDF uses

**Sources combined** (all in `private notes/`):

| Source | What this plan takes from it |
| --- | --- |
| `ExcellentPDF-feature-roadmap.md` | Vision for students, teachers and researchers; the four rules (fast, small, easy, vibrant); modes; competitor parity matrix; prioritized features; packs |
| `building earths best pdf application.md` | Ten product principles; feature inventory; AI grounding and safety rules; reliability, security, accessibility, QA and acceptance checklists |
| `Earths_num1_pdf_application.md` | Competitive teardown; tile cache and progressive rendering; ink prediction and smoothing; true redaction; forms, signatures and LTV; Liquid Mode; spatial canvas; local RAG with clickable citations |
| `ExcellentPDF-cross-platform-plan.md` | Model / View / Service split; platform services table; "every phase ends green on all OSes"; risk handling; tablets last |
| `layout and design.md` | Feature folders, budgets as files, packs outside the core, ADR and feature-spec templates |

Facts checked for this plan: SumatraPDF's vendored MuPDF (`C:\dev\sumatrapdf\ext\mupdf`, version 1.28.5 plus 36
patches in `ext/patches/`; upstream SumatraPDF carries 70 on the same base, see `research.md` R1), every MuPDF function named here (present in its 1.28.5 headers), Enhanced's smart dark
mode (`src/PdfDarkMode*.cpp`), and NSH (installed v0.5.0 at `C:\dev\nsh`, source `C:\dev\nsh-src` at tag v0.5.0, Apache-2.0): first 0.4.0, then upstream v0.5.0 and
`main` re-checked on 2026-11-12.

---

## Contents

1. Vision and product promise
2. Principles and the four rules
3. Decisions
4. Licensing
5. Architecture
6. The MuPDF layer
7. The page canvas (native component)
8. Pen and ink engine
9. Data model, storage and files
10. UI, design system and modes
11. Feature plan with priorities
12. AI, search and knowledge
13. Performance budgets and instrumentation
14. Reliability, security and privacy
15. Accessibility
16. Testing and quality
17. Repository layout and engineering rules
18. Phased roadmap
19. What was dropped from the source documents, and why
20. Risks
21. Decisions needed from the maintainer
22. First ten tasks
23. Appendix A: feature → MuPDF API map
24. Appendix B: references
25. Appendix C: lessons from PdfCraft
26. Appendix D: expert review additions
27. Implementation plans by feature tag (every feature has a tag such as **A.2**; look it up here)

---

## 1. Vision and product promise

**Vision.** The best tool a student, teacher or researcher could want: the most complete reader, annotator,
note-taking, study and research app on any platform, and still the fastest to open, the smallest to download and the
easiest to use. Anything people rely on in Samsung Notes, GoodNotes, Notability, OneNote, Xodo, PDF Expert, MarginNote,
LiquidText or Flexcil should exist here, on PDFs and on blank notebooks alike.

**Promise.** Open any document quickly, understand it deeply, work with it precisely, and find every useful thing
again.

**The market gap.** No product combines all of these:

1. SumatraPDF's speed and small size
2. PDF-XChange's annotation and editing depth
3. GoodNotes' pen feel
4. LiquidText's excerpt workspace
5. A modern, calm, colorful interface
6. One codebase across Windows, macOS, Linux, iPad and Android tablets
7. Offline, no account, open file formats

| Competitor | Strength we match | Weakness we avoid |
| --- | --- | --- |
| Adobe Acrobat | Full ISO feature set | Subscription, slow start, heavy background services, cloud lock-in |
| PDF-XChange Editor | Annotation depth, measurement, OCR | Windows-only, dated UI |
| Foxit | Feature completeness | Upselling, telemetry, memory use |
| Apple Preview | Instant launch, clean UI | Mac-only, damages forms, weak annotations |
| SumatraPDF | Tiny, instant | Read-mostly, Windows-only |
| LiquidText / MarginNote | Spatial excerpts | Proprietary formats, slow on large files |
| GoodNotes / Notability | Pen feel, audio sync | Weak PDF standards support, flattens annotations |

**Our edge:** speed, size, many formats, offline, no account, open formats, pen-first on desktop and tablet, and an
interface that makes studying feel good.

**North star:** invisible when reading, extraordinarily capable when needed, never trading capability for speed.

---

## 2. Principles and the four rules

### 2.1 The four rules (every feature must pass all four)

1. **Blazingly fast.** Hard budgets enforced in CI (§13). Feature code does not run at startup unless the feature is
   visible. Heavy modules (OCR, AI, dictionaries, handwriting, math rendering) load on first use.
2. **Small.** Core installer ≤ 15 MB. Everything else is a downloadable, signed, removable **pack** (fonts,
   dictionaries, OCR and handwriting models, local AI models, voices, templates, stickers).
3. **Easy.** Progressive disclosure: reading is clean by default; pen tools appear when a stylus touches or `P` is
   pressed; teacher tools appear in Present mode; specialist tools stay in Advanced. Every action is reachable three
   ways: toolbar or menu, keyboard shortcut, command palette.
4. **Vibrant but readable.** Colorful accents and joyful motion in the chrome; the document stays the hero and stays
   neutral unless the user picks a paper color or dark mode. WCAG AA contrast enforced in CI.

### 2.2 Product principles

1. **Local-first.** Reading, annotating, search and file management work offline with no account.
2. **Trustworthy.** Source files are never changed without an explicit save. Lossy conversions, redaction, signature
   effects and AI uncertainty are explained before they happen.
3. **Recoverable.** Autosave, undo, crash recovery, version history and backups are part of the architecture, not
   features added later.
4. **Interoperable.** Standard PDF annotations with appearance streams, so other readers show our work. No mandatory
   proprietary format.
5. **Evidence-linked AI.** Answers about documents link to exact pages and passages, and separate quotes, extracted
   facts, inference and uncertainty.
6. **Accessible.** Keyboard, screen readers, high contrast, UI scaling, reduced motion on every platform.
7. **Measure before optimizing.** Instrument first; optimize what the numbers show.

---

## 3. Decisions

| # | Decision | Choice | Why |
| --- | --- | --- | --- |
| D1 | Language | Rust (edition 2024, stable toolchain) | Memory safety, one language for core and UI, NSH is Rust |
| D2 | UI framework | NSH, Rust components (not TSX) | Native views on all five platforms, unstyled accessible base components, Rust apps need no QuickJS |
| D3 | Document engine | MuPDF 1.28.5 as vendored by SumatraPDF, with its patches | Same rendering, format coverage and fixes as SumatraPDF; also gives annotations, redaction, forms, signatures, text, OCR and writers |
| D4 | Bindings | Our own `excellent-mupdf-sys` built from Sumatra's tree, C shims around every call | Keeps Sumatra's patches; MuPDF's `fz_try` uses `setjmp`/`longjmp`, which must not cross Rust frames |
| D5 | Page view | One native component (`excellent.page-canvas`) per platform, shared Rust core | NSH has no document view and no pen pressure; ink latency needs direct OS input |
| D6 | Annotation storage | Inside the PDF (standard annotations, incremental save); sidecar only for non-PDF formats and app-only data | Interoperability |
| D7 | App data | SQLite (`rusqlite`) with FTS5 for library, index and notes | Proven, offline, single file, fast full-text search |
| D8 | Optional features | Packs, downloaded on first use | Size rule |
| D9 | AI | Optional, provider-agnostic, local-first (llama.cpp or Ollama pack), cloud only with explicit consent | Privacy principle |
| D10 | First platform | Windows, then macOS and Linux, then iPad and Android tablets, web last | Users and the existing Enhanced build are on Windows; tablets need desktop parity first |

---

## 4. Licensing

- **MuPDF is AGPL-3.0** (or a paid commercial licence from Artifex). Using it means ExcellentPDF is distributed under
  the AGPL with full source, unless a commercial MuPDF licence is bought.
- **NSH is Apache-2.0**, which may be combined into an (A)GPLv3 program.
- **SumatraPDF is GPLv3.** GPLv3 §13 allows combining with AGPLv3 code, so Enhanced's C++ logic (dark mode, pen tools,
  vocabulary, study export) may be ported into the Rust code. Check SumatraPDF's exact licence terms before copying.
- **AGPL §13 (network use):** the web version, or any server that runs MuPDF for users, must offer those users the
  source. Desktop and tablet apps just ship source as usual.
- **Consequence:** the "open core / commercial dual licence" idea in `Earths_num1_pdf_application.md` works only with a
  commercial MuPDF licence. This is decision **Q1** in §21.
- Every dependency goes through `cargo deny` (licences, advisories, duplicates) in CI.

---

## 5. Architecture

### 5.1 Layers

```
+--------------------------------------------------------------------------------+
|  App shell (NSH Rust components): window, title bar, menus, DockArea layout,  |
|  tabs, sidebar, inspector, command palette, dialogs, home, settings           |
+--------------------------------------------------------------------------------+
|  Feature views (NSH components, one folder per feature)                        |
|  annotations ; ink tools ; notebooks ; study ; present ; search ; library ; AI |
+--------------------------------------------------------------------------------+
|  Feature models (pure Rust, no UI or OS types, unit-tested)                    |
+--------------------------------------------------------------------------------+
|  Document core: documents, pages, layout, text, selection, render scheduler,  |
|  tile cache, dark mode, annotation model, undo, save                           |
+--------------------------------------------------------------------------------+
|  excellent-mupdf (safe Rust API)  →  excellent-mupdf-sys (C shims + bindgen)  |
|  →  MuPDF 1.28.5 + Sumatra patches + thirdparty (freetype, harfbuzz, ...)     |
+--------------------------------------------------------------------------------+
|  Page canvas: shared Rust core + one thin native component per platform      |
|  (Win32/D3D/D2D ; AppKit/Metal ; GTK 4 ; UIKit ; Android View)                |
+--------------------------------------------------------------------------------+
|  Platform services (traits; one implementation per OS): files, dialogs,      |
|  print, clipboard, TTS, key store, notifications, shell, single instance      |
+--------------------------------------------------------------------------------+
```

Rule: each layer talks only to the layer directly below it. Feature models never touch MuPDF or the OS directly; they
go through the document core and the platform services.

### 5.2 What NSH provides and what we build

| Need | NSH piece | Notes |
| --- | --- | --- |
| Window, title bar, menu bar | `nsh/desktop` window and menus | App-owned title bar is possible |
| Dockable panels, splits, tab groups | `DockArea`, `ResizablePanelGroup` | Layout is data, so saved workspaces are serializable |
| Tabs, toolbars, toggles | `Tabs`, `Toolbar`, `ToggleGroup` | Roving focus is built in |
| Command palette | `Command` | Virtualized, filtered natively |
| Outline, bookmarks, file tree | `Tree` | Virtualized |
| Thumbnails, search results, annotation inbox | `virtual_list` (nsh-core), `DataTable` | |
| Notes and AI answers | `RichEditor` (ProseMirror-style kernel), `TextView` (Markdown / HTML) | |
| Markdown / code source editing | `Editor` | |
| Menus, context menus, popovers, tooltips, sheets | `Menu`, `ContextMenu`, `Popover`, `Tooltip`, `HoverCard`, `Sheet` | |
| Dialogs and confirmations | `Dialog`, `AlertDialog` | |
| Pen color and width pickers | `ColorPicker` (RGB, HSL, HSB, OKLCH), `Slider` | |
| Study statistics | `Plot` (`Line`, `Area`, `Pie`, ...) | |
| Calendars, due dates | `Calendar`, `DatePicker` | |
| Theme tokens | shadcn/ui-style theme (`theme.rs`) | We add mode accents (§10.3) |
| Shortcuts | `keymap` | |
| Drag and drop of files | file drop | Reports each file's MIME type (v0.5.0) |
| Several windows | `open_window`, `WindowRef`, `main_window` (v0.5.0) | Each window paces its own frames on the shared UI thread; used by V.11 and W.8 |
| Tray and system-wide shortcuts | `set_tray`, `register_global_shortcut` in `nsh_core::element::desktop` (v0.5.0) | Used by N.10 |
| Translations and locale data | `nsh-i18n` (rust-i18n) (v0.5.0) | Used by U.7 |
| Updates | NSH updater: restarts into the installed app, `shellInfo()` reports versions (v0.5.0) | K.25 checks it against our signing and rollback needs |
| Accessibility | UI Automation, AT-SPI, NSAccessibility, iOS / Android | Our canvas must expose its own tree (§15) |
| Headless UI tests | `nsh test`, `nsh-testing` | |
| **Document view** | — | **We build it** (§7) |
| **Pen pressure, tilt, eraser end** | Only the pointer type (`PointerType::Pen`, v0.5.0) | **We build it** inside the canvas (§8), reading the pen from the OS as SumatraPDF Enhanced does |

NSH versions: this plan targets **v0.5.0** (tagged 2026-11-11). Upstream `main` moves fast (about 670 commits in the
week after 0.4.0), so pin a tag, and check the upstream log before each phase for features that replace our own code.
Changes on `main` after v0.5.0 that we want are marked "NSH main" below.

### 5.3 Threads

```
 main / UI thread            NSH engine thread            render workers (N = cores − 1, bounded)
 ────────────────            ─────────────────            ───────────────────────────────────────
 OS input, canvas blit  ←──  app state, components   ──→  MuPDF: display lists, tiles, text,
 ink preview (same frame)    reactive signals             search, thumbnails (cloned fz_context)
            │                                                        │
            └──── canvas core (Rust): tile cache, scheduler ←────────┘
                                                                     │
                                       background workers: index, OCR, embeddings, save, packs
```

- The UI thread never waits for the engine thread or a worker. Pen strokes are drawn by the canvas on the UI thread
  in the same frame they arrive, then sent to the model.
- MuPDF: one base `fz_context` created with `fz_new_context` and real `fz_locks_context` lock functions; each worker
  uses `fz_clone_context`. An `fz_document` is not thread-safe, so each document has a mutex for page loading and
  edits. Display lists (`fz_new_display_list_from_page`) are immutable and are rendered from several workers at once
  (`fz_run_display_list`). Sumatra's patch `0007-thread-local-secret-contexts` is required for this.
- Cancellation: every render and text job carries an `fz_cookie`; setting `abort` stops it. Jobs for pages that left
  the viewport are cancelled at once.

### 5.4 Workspace crates

```
crates/
  excellent-mupdf-sys/   build.rs (cc) compiles Sumatra's ext/mupdf + amalgamated thirdparty; C shims; bindgen
  excellent-mupdf/       safe API: Context, Document, Page, DisplayList, Pixmap, StextPage, Annot, Writer
  excellent-core/        document model, layout (single / continuous / facing / book), coordinate transforms,
                         selection, render scheduler, tile cache, dark mode, undo bridge, save
  excellent-canvas/      platform-neutral canvas: viewport, gestures, hit testing, overlays, ink capture model
  excellent-canvas-win/  Win32 child window, DXGI flip-model swap chain, Direct2D, WM_POINTER pen input
  excellent-canvas-mac/  NSView + CAMetalLayer, tablet events
  excellent-canvas-gtk/  GTK 4 widget, GtkGestureStylus axes
  excellent-canvas-ios/  UIView, UITouch force / altitude / azimuth, coalesced and predicted touches
  excellent-canvas-android/ JNI side of a Java/Kotlin view (MotionEvent axes, front-buffered rendering)
  excellent-ink/         stroke model, smoothing, prediction, variable-width outlines, appearance streams
  excellent-text/        text layer (K.13): reading order, words and lines, selection, highlight and copy
  excellent-store/       SQLite: library, FTS5 index, notes, settings, migrations
  excellent-platform/    service traits + win / mac / linux / ios / android implementations
  excellent-packs/       download, verify (Ed25519), install, remove, offline use
  excellent-ai/          provider trait, local (llama.cpp / Ollama) and cloud adapters, RAG, citations
  excellent-features/    one module per feature: model + view + tests
  excellent-app/         the NSH app: root component, shell, wiring, #[native_components(...)] per platform
  excellent-cli/         batch tools (merge, split, convert, OCR, redact) on the same core
```

---

## 6. The MuPDF layer

### 6.1 Source and build

- **Source:** **upstream SumatraPDF's** vendored tree (`sumatrapdfreader/sumatrapdf` `master`, the `upstream` remote
  of `C:\dev\sumatrapdf`): MuPDF tag `1.28.5` (commit `8ad45e92f`) plus its **70 patches** (`0001`–`0087`, with
  gaps), byte-for-byte (verified there by `ext/patches/README.md`'s procedure). Enhanced's own tree is the same
  base with only 36 of them; upstream's extra 40 bring crash fixes, certifying signatures, RTL text, EPUB and SVG
  work and speed-ups (`research.md` R1.4).
- **Plus Enhanced's six annotation patches,** renumbered `0088`–`0093` because upstream reuses their numbers for
  different patches: `0048` snapshot keeps undo → `0088`, `0049` rounded highlight corners → `0089`, `0050`
  dotted borders → `0090`, `0051` pinned note → `0091`, `0052` arrowheads → `0092`, `0053` filled arrowheads →
  `0093`. Done in T2 (2026-11-13): all six apply on upstream's tree once `0091` and `0093` are converted from CRLF
  to LF (the 2026-11-12 "conflict" was line endings) and `0093`'s stray byte-order-mark hunk is dropped. Upstream's
  own `0053` had to be regenerated (it was a diff against its previous version). Details and the byte check:
  `third_party/mupdf-patches/README.md`.
- **Our-own MuPDF sources** that SumatraPDF compiles into its `mupdf` project live outside the tree: upstream has
  moved them to `src/shared/mupdf/` (`load-jxl`, `noto_sumatra`, `mupdf_load_system_font.c`, `pkcs7-windows`) and
  added a macOS font loader, `src/ng/mupdf/mupdf_load_system_font_mac.c`. Copy them from there.
- **Copy, don't link to the Sumatra checkout.** Vendor the tree into `third_party/mupdf/` with the patches in
  `third_party/mupdf-patches/`, and keep Sumatra's rule: every change to the vendored tree is a patch, in the same
  commit. `third_party/mupdf-patches/README.md` records each patch's origin (upstream SumatraPDF, Enhanced, or ours)
  and the commit it came from. Track upstream SumatraPDF's updates and pull new patches across.
- **Thirdparty libraries:** reuse upstream SumatraPDF's amalgamations (its `a-brotli` is already newer than
  Enhanced's, which covers what MuPDF 1.28.6 changed) where they exist (`ext/a-freetype`, `a-harfbuzz`,
  `a-jbig2dec`, `a-lcms2`, `a-openjpeg`, `a-brotli`, `a-libwebp`, `a-mujs`, `a-extract`, `a-zlib`, `a-gumbo`) —
  one `.c` per library is much easier to build from `build.rs` than upstream build systems. Plus `libjpeg-turbo`,
  `cmark-gfm` and `jxldec` (needed by patch `0047`), and, since all of MuPDF is built (`dev_decisions.md` DD1),
  `tesseract`, `leptonica`, `zxing-cpp`, `zint`, `a-libarchive`, `a-bzip2` and `liblzma`.
- **Build:** the `cc` crate from `build.rs`, per target (MSVC, Apple clang, gcc / clang, Android NDK, iOS SDK).
  Everything upstream SumatraPDF compiles into MuPDF is built on every target, including OCR, barcodes, archives
  and the OFD reader; the per-OS choices (fonts, signatures, JPEG XR) and the gaps are in `dev_decisions.md` DD1.
  `bindgen` runs at development time and the generated bindings are committed, so users don't need libclang.
- **Reference implementation:** the `mupdf` crate (mupdf-rs) solves the same problems (C shims for `fz_try`,
  context cloning, build scripts); read it before writing ours. Pin it as a fallback, not a dependency.

### 6.2 Which Sumatra patches to carry

| Keep on all platforms | Windows-only (behind `cfg`) | Only if we ship a `mutool`-style CLI |
| --- | --- | --- |
| 0007 thread-local contexts; 0008 uncaught throw report; 0009 CP1250 forms; 0010 / 0011 ToUnicode fixes; 0012 external file streams; 0013 / 0014 / 0015 HTML / XML / CSS robustness; 0019 freetype zlib + brotli; 0025 / 0027 WebP; 0032 / 0033 annotation appearances; 0036 OCG visibility; 0038 CSS background images; 0039 empty Markdown; 0040 / 0044 SVG; 0047 JPEG XL; upstream 0048 / 0050–0064 / 0068–0074 / 0077–0087 (compress option, crash fixes, JavaScript errors, DocMDP signing, EPUB, SVG, HTML / CSS, RTL text, password by key, speed-ups); Enhanced's 0088 snapshot keeps undo; 0089–0093 annotation appearance polish | 0006 JPEG-XR via WIC; 0004 / 0045 console I/O | 0001 / 0002 tool usage and options; 0005 / 0076 pdfinfo to buffer; 0042 / 0043 / 0049 merge; 0046 / 0065 grep; 0066 / 0067 `run` scripts; 0075 pdfbake |

Patch `0003` (signatures through Windows CryptoAPI) is replaced by a cross-platform signer written in Rust (§6.4).

### 6.3 Safe Rust API rules

- Every MuPDF call goes through a C shim that runs it inside `fz_try` / `fz_catch` and returns an error code and
  message. No `longjmp` ever crosses a Rust frame.
- Owned MuPDF objects are Rust types with `Drop` calling the matching `fz_drop_*`. Contexts are `!Send`; documents are
  shared behind a mutex; display lists are `Send + Sync`.
- Geometry stays typed: page points vs device pixels are different types, with the page's rotation and crop box
  applied in one place (`excellent-core::transform`).
- Every `unsafe` block has a `// SAFETY:` comment (NSH's rule, adopted).

### 6.4 What MuPDF gives us (so we don't write it)

| Need | MuPDF | Replaces in the old blueprint |
| --- | --- | --- |
| Open PDF, XPS, EPUB, MOBI, FB2, CBZ, SVG, images, HTML, Markdown | `fz_open_document` | Custom parser, XRef reconstructor, mmap I/O layer |
| Fast repeated rendering | `fz_new_display_list_from_page` + `fz_run_display_list` with a scissor rect (tiles) | PDFium tile FFI |
| Text, selection, search | `fz_new_stext_page_from_page`, `fz_search_stext_page` | Custom SIMD search |
| Annotations with appearance streams | `pdf_create_annot`, `pdf_update_annot`, `pdf_set_annot_appearance_from_display_list` | Custom `/AP` generator |
| Undo / redo | `pdf_enable_journal`, `pdf_begin_operation`, `pdf_undo`, `pdf_redo` | Custom undo for PDF edits |
| Incremental save | `pdf_save_document` with `pdf_write_options.do_incremental` | Custom incremental writer (which had bugs) |
| True redaction | `pdf_apply_redaction` with `pdf_redact_options` (`black_boxes`, `image_method`, `line_art`, `text`) | Custom operator stripping |
| Forms and form JavaScript | AcroForm API, `pdf_enable_js` (MuJS) | QuickJS sandbox |
| Signatures | `pdf_sign_signature`, `pdf_count_signatures`, signer / verifier callbacks | — (we supply the crypto) |
| OCR | `fz_new_ocr_device`, `fz_new_pdfocr_writer` (with Tesseract; data as a pack) | — |
| Export | `fz_new_document_writer` (PDF, PNG, SVG, text, HTML, ...); Artifex `extract` for DOCX / ODT | — |
| Reflow | structured text, HTML / CSS layout engine | Part of Liquid Mode |

**Formats beyond MuPDF** that Sumatra reads (DjVu via `djvudec`, CHM via `chmdec`, CBR via UnRAR, HEIC / JXL / AVIF
images) come later as separate engine crates behind the same `Document` trait. Check each library's licence first;
UnRAR's licence has extra conditions.

---

## 7. The page canvas (native component)

### 7.1 Why a native component

NSH draws the app's own views but has no document view. Since v0.5.0 its touch events say whether a pen, finger or
mouse made them, but they still carry no pen pressure, tilt, rotation or eraser state. NSH's
answer for content like this is a **native component**: a view the app's crate registers by name with
`#[native_components(windows|macos|linux|ios)]`; on Android it is registered from Java or Kotlin. The app places it
like any element (`native("excellent.page-canvas")`), sends JSON props, gets events, and calls commands through an
`element_ref`.

### 7.2 Shared core, thin platform layers

Almost all canvas logic is platform-neutral Rust in `excellent-canvas`:

- viewport, zoom, rotation, layouts (single, continuous, facing, book, horizontal, right-to-left, grid)
- the tile scheduler and cache (§7.4)
- hit testing for links, text, annotations and form fields
- text selection (word, line, paragraph, region; across columns)
- overlays: selection, search hits, annotation handles, ink preview, laser pointer, reading ruler
- gesture interpretation (pan, pinch, double-tap zoom, palm rejection rules)

Each platform layer only does three things: present pixels, deliver raw input with full pen data, and expose an
accessibility tree.

| Platform | View | Presenting | Pen input |
| --- | --- | --- | --- |
| Windows | Child `HWND` (`NativeComponent::window`) | DXGI flip-model swap chain, Direct2D / Direct3D 11, waitable swap chain for low latency | `WM_POINTER*`, `GetPointerPenInfo` (pressure, tilt, rotation, eraser, barrel button), `GetPointerPenInfoHistory` for coalesced points |
| macOS | `NSView` | `CAMetalLayer` | `NSEvent` pressure and tilt (tablets) |
| Linux | GTK 4 widget | `GtkSnapshot` with textures (GL where available) | `GtkGestureStylus` axes (pressure, x / y tilt) |
| iPad | `UIView` | `CAMetalLayer` | `UITouch` force, altitude, azimuth; coalesced and predicted touches |
| Android | Java/Kotlin `View` calling Rust over JNI | `SurfaceView`; evaluate androidx front-buffered rendering for ink | `MotionEvent` pressure, `AXIS_TILT`, `AXIS_ORIENTATION`, historical samples, `requestUnbufferedDispatch` |

### 7.3 The Windows constraint (important)

On Windows, NSH offers two kinds of native component:

- **Drawn** (`draw` into NSH's Direct2D context): NSH content can overlay it, but **its container gets the pointer
  input**, so we would get no pen data and add a hop through the engine thread.
- **Own window** (`window`, a child `HWND`): it gets its own input (pen included) and can run a low-latency swap chain,
  but **it covers any NSH content drawn over it** (popovers, tooltips, in-window menus).

**Plan:** use the child window. Draw everything that must overlap the page inside the canvas (selection handles, ink,
search hits, the floating selection toolbar, the laser), and place NSH popovers beside the page rather than on top.
NSH's context menus via `Menu.popup` are native Win32 menus and are not affected.

**Pen pressure does not wait for NSH.** The child window receives `WM_POINTER*` messages itself, so the canvas reads
pressure, tilt, rotation, the barrel button and the eraser end with `GetPointerPenInfo` and
`GetPointerPenInfoHistory`, exactly as SumatraPDF Enhanced does in its page window (`src/PointerInput.h`, the
`WM_POINTER` handling in `src/Canvas.cpp`). Port `PointerInput.h` to Rust in K.8. Enhanced saves one mean width per
stroke because a PDF `/Ink` annotation has one width; we keep the per-point widths and save the outline as the
appearance stream (§8.3), so every reader shows the pressure.

**Also:** propose upstream to NSH pen pressure, tilt, rotation and eraser state on its pointer events (v0.5.0 already
reports the pointer type) and input for drawn components. If it lands, the drawn mode becomes possible and overlays get simpler. Track this as task
**T9**.

**The compositor draws the newest ink (added in T8).** The swap chain is shown through DirectComposition, and on
Windows 11 24H2 and later the canvas hands every pen sample to a delegated ink trail
(`IDCompositionDelegatedInkTrail`) the moment it arrives. Windows draws those points on its next refresh, without
waiting for the app's frame; when our frame that contains them is presented, the trail lets go of them. This is the
mechanism Edge and Chrome use for low-latency web ink. Older Windows (Chromium found the trail flickers before 24H2)
falls back to the app's own frame, drawn as soon as the swap chain takes one.

### 7.4 Rendering pipeline

1. **Open:** load the document, page count and page sizes only. Show the first page as soon as it is ready; never
   render every page on open.
2. **Display lists:** build one per page on a worker when the page comes near the viewport; keep the most recent ones
   in an LRU cache with a memory budget.
3. **Tiles:** 256 × 256 device pixels, rendered at a zoom bucket: the exact scale in steps of 1/4096, so a tile is
   drawn at the size it was rendered and text stays sharp. While a new zoom's tiles render, the old zoom's tiles of the
   same page are drawn under them, stretched, so the page never goes blank. (Changed in T7: coarse buckets, stretched
   to the exact scale, blurred text.)
4. **Progressive:** draw a low-resolution page bitmap first, then sharp tiles; while pinch-zooming, scale the existing
   tiles and re-render when the gesture settles.
5. **Priority** (highest first): visible tiles → current page's text geometry → next / previous screen → visible
   thumbnails → background work (index, OCR, embeddings) → bulk jobs. Background work pauses while the user scrolls or
   writes.
6. **Cache key:** document id + revision, page, zoom bucket, rotation, crop box, color mode, dark-mode profile hash,
   annotation-visibility flag, renderer version.
7. **Memory:** tile cache and display-list cache each have a budget; evict least recently used first, and drop
   everything off-screen under OS memory pressure.
8. **Annotations while editing:** render the page without the annotation being edited and draw that one live, so
   edits don't re-render the page.

### 7.5 Dark mode and reading themes

- **Port Enhanced's object-level smart dark mode** (`PdfDarkMode*.cpp`): it maps text and vector colors to the theme
  through OKLab, classifies each image (photo, light-background artwork, icon or line art, full-page scan) and decides
  per image whether to preserve, adapt or recolor it, with special handling for scanned pages. This is far better than
  the per-pixel saturation shader in `Earths_num1_pdf_application.md`, which inverts light graphics and leaves dark
  text on colored backgrounds.
- Themes: light, dark, sepia, OLED black, high contrast, custom; per-document overrides; paper colors for notebooks.
- Reflowable formats (EPUB, FB2, HTML) get theme CSS instead of recoloring, as Enhanced does.

---

## 8. Pen and ink engine

### 8.1 Latency target

**Pen-down to pixels within one display frame** (≤ 8.3 ms at 120 Hz, ≤ 16.7 ms at 60 Hz), measured per platform.
(The "< 4.2 ms" in the blueprint is shorter than one refresh and cannot be met.)

### 8.2 Capture to stroke

1. **Capture:** every hardware sample, including coalesced ones, with position, pressure, tilt, azimuth, timestamp and
   tool (pen tip, eraser end, finger, mouse).
2. **Preview:** draw the new segment in the same frame on the canvas's own surface, before anything else sees it.
3. **Prediction:** extend the preview by one or two frames with the OS prediction where it exists (iOS predicted
   touches, Android's motion predictor), otherwise a small constant-velocity / Kalman extrapolation in Rust. Predicted
   points are replaced when real samples arrive.
4. **Smoothing:** Catmull-Rom through the samples, converted to cubic Béziers
   (B0 = P1, B1 = P1 + (P2 − P0) / 6, B2 = P2 − (P3 − P1) / 6, B3 = P2), with light input filtering per pen type.
5. **Width:** from pressure (per-pen response curve, e.g. w = min + (max − min) × p^1.25), tilt (pencil shading,
   angle-aware calligraphy) and speed.
6. **Commit on pen-up:** one undoable operation.

### 8.3 Saving ink as standard PDF

- **InkList** holds the stroke's points (`pdf_add_annot_ink_list`), so every reader knows the path.
- **Appearance stream:** constant-width pens can use MuPDF's own (`pdf_update_annot`). Pressure-varying pens get our
  own appearance: the stroke's outline as a filled Bézier path, built into a display list and attached with
  `pdf_set_annot_appearance_from_display_list`. It must use the smoothed curves; the blueprint's sample wrote raw
  points as straight lines and lost the smoothing.
- The original samples (pressure, timestamps) go into a private key on the annotation, so we can re-render, replay and
  sync audio later. Other readers ignore it.
- Highlighters use blend mode Multiply.

### 8.4 Tools

Fountain pen, ballpoint, pencil (texture, tilt shading), brush, calligraphy (angle-aware), marker, highlighter
(multiply), tape (hide / reveal); erasers by stroke, by area, highlighter-only, and the stylus's eraser end; lasso to
move, resize, rotate, duplicate, recolor, change width, copy as image and convert to text; hold-to-snap shapes; ruler
and protractor; favorite-pens bar; palm rejection and a "finger draws" toggle.

Tags: capture **H.1**, pens **H.2**, highlighter **H.3**, tape **H.4**, erasers **H.5**, lasso **H.6**, shapes
**H.10**, ruler and protractor **H.11**, favorites **H.7**, palm rejection **H.8**, saving **H.9** (plans in §27.7).

---

## 9. Data model, storage and files

### 9.1 Principles

- Source content stays separate from annotations and notes; derived caches stay separate from user data.
- Every document, page, annotation, note and canvas object has a stable id.
- Annotation coordinates are stored in normalized page space, correct under rotation, crop box and mixed page sizes.
- Unknown annotation types are preserved, never dropped.
- Schema migrations are explicit and tested.

### 9.2 Where data lives

| Data | Where | Format |
| --- | --- | --- |
| PDF annotations, form values, page edits | Inside the PDF, incremental save | Standard PDF |
| Annotations on EPUB, DjVu, CBZ, etc. | Sidecar next to the file, or in app data for read-only locations | Documented JSON |
| Notebooks | `.expdf` = zip of PDF + JSON sidecar + assets (audio, images) | Documented; exports to plain PDF |
| Library, tags, collections, read status | `library.db` | SQLite |
| Full-text and annotation index | `library.db` | SQLite FTS5 |
| Embeddings (when AI is on) | `vectors.db` or files beside it | SQLite / HNSW (`usearch`), rebuildable |
| Settings | Text file (`settings.toml`) | Portable mode keeps it next to the exe |
| Flashcards and review history | `library.db` | SQLite, FSRS scheduling |
| Recovery journal | App data | Append-only, fsync'd |
| Workspaces, tabs, view states, closed tabs | `library.db` (tables in §11.16) | SQLite; tab thumbnails as small WebP files in the cache folder |

### 9.3 Saving and recovery

- **Autosave** writes a journal of operations immediately and saves the PDF incrementally after idle time, on tab
  switch and on close.
- **Atomic writes:** full saves go to a temporary file, are verified (reopened), then renamed over the original.
- **Crash recovery:** on start, replay the journal over the last saved revision; Sumatra's patch `0048` keeps MuPDF's
  journal intact across snapshots.
- **External change detection:** reload safely when the file changes on disk; on a conflict, offer to keep both.
- **Version history** for app-managed files (notebooks) by keeping incremental revisions.

---

## 10. UI, design system and modes

### 10.1 Layout

**The look is Vivaldi's** (maintainer's reference, 2026-11-13: `dev notes/images/ui-reference-vivaldi-layout.jpg`):

- **Tab strip at the very top,** in the title bar as Vivaldi's is: document tabs (with tab groups), a new-tab button,
  and the window controls at the right end. Its **leftmost small button switches workspaces** (W.1).
- **Tab-list button at the right end of the tab strip,** just before the window controls, as Vivaldi has (maintainer's
  reference, 2026-11-13: `dev notes/images/ui-reference-vivaldi-tab-list.png`). A chevron that opens a dropdown of
  every tab: a "Search all tabs" field with a "…" menu, then collapsible sections with counts: **Open tabs**,
  **Inactive tabs** (unloaded, W.14), **Duplicate tabs** (the same file open more than once, each with "N copies" and
  its workspace badge, plus "Close all duplicate tabs") and **Recently closed tabs**. Built with R.12 (step 4).
- **Optional icon rail on the left edge,** like Vivaldi's panel bar: a narrow column of function icons (library,
  thumbnails, outline, bookmarks, search, annotations, …) that opens the matching panel; the user can hide it.

- **Top bar:** under the tab strip: title, global search, mode switcher, key actions.
- **Left sidebar:** the panel the icon rail opens: library, thumbnails, outline, bookmarks, search results,
  attachments, annotations.
- **Center:** the page canvas.
- **Right inspector:** properties, comments, notes, citations, AI, extracted tables.
- **Bottom dock (optional):** annotation inbox, jobs (OCR, export, index), comparison.
- **Command palette** for every action, with shortcut hints.
- All panels resize, collapse and reorder; each workspace (§11.16) remembers its own layout (`DockArea` layout data).
- **Workspace switcher:** the small button at the left end of the tab strip.
- Contextual toolbar near a selection, never covering it.

### 10.2 Modes, not menus

| Mode | Accent | Toolbar shows |
| --- | --- | --- |
| Read | Blue | Zoom, layout, find, theme, read aloud |
| Annotate | Amber | Markup, comments, shapes, stamps, measure |
| Write | Violet | Pens, erasers, lasso, shapes, page templates |
| Study | Green | Flashcards, review queue, quiz, timer |
| Present | Coral | Laser, spotlight, magnifier, whiteboard, presenter view |

Each mode has its own minimal, customizable toolbar (drag and drop, shareable presets). The active tab and toolbars
take the mode's accent.

### 10.3 Design system

- **Tokens** (on NSH's shadcn-style theme): background, surface, raised surface, text, muted text, border, focus,
  accent, success, warning, danger, plus mode accents and annotation colors.
- **Theme families:** neutral light / dark / high contrast, plus vivid families (Aurora, Coral, Lagoon, Citrus, Grape,
  Forest), each in light, dark and high contrast. Themes are small shareable files.
- **Reconciling "calm" and "vibrant":** neutral chrome and document area; color lives in accents, icons, pen palettes,
  covers and celebrations.
- **Typography:** one bundled UI font in the core (larger font sets are packs); optional serif and dyslexia-friendly
  fonts for reflow.
- **Icons:** original duotone vector set that recolors with the theme; pen nibs show their ink color.
- **Motion:** short and purposeful (tool switches, page turns, correct / incorrect feedback, streaks), 60 fps, never
  delays input, respects reduced motion. Pointer-driven motion follows the pointer in the same frame (NSH's rule).
- **Density:** compact and comfortable modes; "Essential" and "Advanced" without splitting the product.
- **States:** focus, hover, pressed, disabled, selected, destructive, loading skeletons, empty states that teach one
  next step.
- **First run:** short guide and contextual tips, skippable and replayable.

---

## 11. Feature plan with priorities

Priority: **P0** next; **P1** strong differentiator; **P2** valuable; **P3** later.
Every feature starts from a spec (`docs/features/_template.md`: problem, user story, UI sketch, mode, speed and size
impact, acceptance tests).

### 11.1 Parity with SumatraPDF Enhanced (P0 baseline)

Already in Enhanced and must exist before the Rust app replaces it: multi-format reading, tabs and tab groups, command
palette, smart dark mode, pen tools with pressure and nib types, annotation edit / filter / search / transfer /
recovery, forms, signing with EUTL trust, merge, CAD viewer, citation and equation hover, offline dictionary and
vocabulary practice, read aloud, translate, Google Lens, AI chat panel, Markdown editing with math and Mermaid, study
export, reading bar, auto-scroll, laser, screenshots, SyncTeX. The Enhanced changelog
(`sumatrapdf/SumatraPDF_Enhanced_CHANGELOG.md`) is the checklist.

Tags for these features (plans in §27): multi-format reading **R.28**, **R.29**; tabs **R.12** and tab groups
**R.13**; command palette **K.16**; smart dark mode **R.14**; pen tools **H.1–H.9**; annotation edit **A.12**,
**A.13**, filter and search **A.14**, transfer **A.16**, recovery **A.15**; forms **O.11**; signing **O.14** with
EUTL trust **O.15**; merge **O.2**; CAD viewer **R.26**; citation and equation hover **R.25**; offline dictionary
**S.6**; vocabulary practice **S.7**; read aloud **U.5**; translate **S.16**; Google Lens **R.31**; AI chat panel
**I.3**; Markdown editing **N.14**; study export **S.8**; reading bar **R.20**; auto-scroll **R.21**; laser
**G.2**; screenshots **O.23**; SyncTeX **R.27**.

### 11.2 Reading and navigation

- P0 **R.1** Layouts: single, continuous, facing, book (cover-aware), horizontal, grid; **R.2** fit page / width /
  height and zoom; **R.3** RTL and manga direction; **R.4** rotation and crop-box view.
- P0 **R.5** Thumbnails; **R.6** page labels and go to page; **R.7** links.
- P0 Document outline (view and **edit**) and personal bookmarks, see §11.14 (**B.1–B.13**).
- P0 **R.8** Back / forward history per tab; **R.9** session restore; **R.10** recent and pinned files; **R.11**
  drag and drop, file associations, single instance and command line.
- P0 **R.12** Tabs; P1 **R.13** named tab groups.
- P0 **R.14** Smart dark mode; **R.15** reading themes (§7.5).
- P0 Split view for the same file or different files, in tab groups like VS Code and Notepad++, see §11.15
  (**V.1–V.14**).
- P1 **R.16** Reflow ("Liquid Mode") for PDFs: reading order, columns, tables, figures; typography controls; math as
  LaTeX / MathML where recoverable.
- P0 Workspaces: named sets of tabs and layouts, switched in place like Vivaldi's, see §11.16 (**W.1–W.14**).
- P1 **R.17** Mini-map with note ticks; **R.18** thumbnail drag reordering.
- P1 **R.19** Full-screen reading and focus mode; **R.20** reading ruler; **R.21** auto-scroll.
- P2 **R.22** Bionic reading; **R.23** paper textures; **R.24** two-column auto-detect.
- P0 **R.25** Citation and equation hover; P1 **R.26** CAD drawing view; P1 **R.27** SyncTeX (parity with Enhanced).
- P0 **R.28** EPUB, MOBI, FB2, XPS, CBZ, SVG, images, HTML and Markdown through MuPDF; P2 **R.29** DjVu, CHM and CBR.
- P1 **R.30** Open from a URL with progressive loading; P2 **R.31** visual search (Google Lens).

### 11.3 Annotation (PDF)

- P0 **A.1** Highlight, underline, squiggly, strikeout; **A.2** replace and insert text; **A.3** colors; **A.4** notes
  on highlights.
- P0 **A.5** Sticky notes; **A.6** text boxes and typewriter; **A.7** callouts with leader lines.
- P0 Shapes: **A.8** lines and arrows; **A.9** rectangles and ellipses; **A.10** polygons and clouds.
- P0 **A.11** Margin notes and anchored note cards; **A.12** annotation inspector; **A.13** select, move, resize,
  copy and paste; **A.14** annotation list with filter and search; **A.15** robust autosave and recovery.
- P1 **A.16** Transfer annotations to another version of a document.
- P1 **A.17** Semantic categories (claim, evidence, question, definition, ...); **A.18** tags; **A.19** filtering by
  color / tag / author / date.
- P1 **A.20** Stamps library with dynamic fields; **A.21** comment threads with resolve; **A.22** layers by person or
  purpose.
- P1 **A.23** Measurement: distance, perimeter, area, angle with calibrated scales (metric and imperial).
- P1 **A.24** XFDF / FDF import and export; **A.25** flatten on export; round-trip tested in other readers;
  **A.28** W3C Web Annotation export.
- P2 **A.26** Annotation reports; **A.27** annotation history timeline.

### 11.4 Handwriting and drawing

- P0 **H.1** Ink capture with pressure / tilt / speed curves; **H.2** pen types; **H.3** highlighter; **H.5**
  erasers; **H.6** lasso; **H.7** favorites bar; **H.8** palm rejection; **H.9** ink saved as standard PDF (§8).
- P1 **H.4** Tape (hide / reveal).
- P1 **H.10** Shape recognition (hold to snap); **H.11** ruler / protractor / compass; **H.12** zoom writing pad;
  **H.13** handwriting straightening.
- P1 **H.14** Layers (ink, highlights, text, images: show, hide, lock); **H.15** bulk restyle.
- P1 **H.16** Fill and gradients; **H.17** eyedropper; **H.18** saved palettes (pastel, neon, earth,
  color-blind-safe).
- P2 **H.19** Handwriting beautify; **H.20** pattern brushes; **H.21** symmetry; **H.22** stroke replay.
- P0 **H.23** Searchable handwriting index (offline); P1 **H.24** convert handwriting to text, **H.25** math to LaTeX,
  **H.26** shapes to vectors (packs).

### 11.5 Notebooks

- P0 **N.1** Blank notebooks with procedural paper (blank, lined, college-ruled, grid, dot, Cornell, music, graph,
  isometric, planner): drawn, not stored, so they cost no size.
- P0 **N.2** Insert pages anywhere in a PDF or notebook; **N.3** expand margins for notes.
- P0 **N.4** Open notebook format (§9.2) that exports to plain PDF.
- P1 **N.5** Infinite canvas pages and whiteboards; **N.6** typed text blocks (headings, lists, checkboxes, code,
  tables); **N.7** images, stickers, sticky notes; **N.8** covers and sections; **N.9** links between pages; **N.10**
  quick note from a global shortcut.
- P2 **N.11** Planners and calendars; **N.12** template packs; **N.13** version history per notebook.
- P1 **N.14** Markdown editing with math and Mermaid (parity with Enhanced).

### 11.6 Audio and lectures

- P0 **L.1** Record audio while writing; tap a stroke or word to jump to that moment.
- P1 **L.2** Offline transcription (whisper.cpp pack), searchable and linked to pages; **L.3** import slides and
  condense to notes.
- P2 **L.4** Screen + pen recording to video.

### 11.7 Study

- P0 **S.1** Flashcards from highlights, notes and lasso selections; **S.2** image occlusion; **S.3** cloze deletions.
- P0 **S.4** One spaced-repetition engine (FSRS) for vocabulary and flashcards; **S.5** daily review on the home page.
- P0 **S.6** Offline dictionary; **S.7** vocabulary practice; **S.8** study export (parity with Enhanced).
- P1 **S.9** Quizzes from a chapter (local AI pack or cloud, optional); **S.10** exam mode that hides answers;
  **S.11** Anki `.apkg` import and export; **S.12** goals, timer, statistics, streaks; **S.13** summaries from your
  highlights.
- P1 Excerpt workspace (LiquidText / MarginNote style), see §11.10 (**X.1–X.4**).
- P1 **S.16** Translate a selection (parity with Enhanced), online after consent or with offline packs.
- P2 **S.14** Assignments with due dates; **S.15** language learning (tap word → dictionary, audio, deck; parallel
  translation).

### 11.8 Teaching

- P0 **G.1** Present mode: **G.2** laser with trails, **G.3** spotlight, **G.4** magnifier, **G.5** pen on slides and
  fading ink, **G.6** whiteboard page, **G.7** curtain reveal.
- P0 **G.8** Presenter view on a second screen: current and next page, notes, timer.
- P1 Grading: **G.9** stamps (✓ ✗ ? score), **G.10** rubric panel with scores summed per page, **G.11** comment bank,
  **G.12** batch open and export.
- P1 **G.13** Worksheet maker: fillable fields, answer-key layer.
- P2 **G.14** Live classroom over LAN or a self-hosted relay, no account; **G.15** quick polls.

### 11.9 Search and library

- P0 Full-text search: **F.1** find in the page and document; **F.2** folder and library; results stream while
  indexing continues.
- P0 **F.3** Global search over documents, annotations, handwriting, transcripts and notes (SQLite FTS5).
- P1 **F.4** Operators (`tag:`, `color:`, `author:`, `after:`, `page:>100`, AND / NOT, phrase, wildcard, fuzzy,
  regex); **F.5** saved searches and smart folders.
- P1 Library: **F.6** collections, tags, read status; **F.7** metadata extraction (DOI, ISBN, arXiv); **F.8**
  duplicate detection; **F.9** folder watching.
- P1 **F.10** Citations: DOI / arXiv detection, metadata fetch, BibTeX / RIS; **F.11** Zotero local API; **F.12**
  backlinks and `[[links]]`; **F.13** export to Markdown / Obsidian with deep links.
- P2 **F.14** Graph view; **F.15** PDF compare (text, layout, annotation diff); **F.16** table and figure extraction.
- P0 **F.17** Home page: recent and pinned files, continue reading, daily review.

### 11.10 Spatial research canvas

- P1 **X.1** Drag excerpts, figures, tables and notes onto an infinite canvas; each keeps a link to its exact page
  region, and clicking it jumps back.
- P1 **X.2** Connect, group and label items; argument maps; **X.3** evidence chains across documents.
- P2 **X.4** Export as Markdown, image, PDF appendix or outline.

### 11.11 PDF tools

- P0 **O.1** Page organizer: insert, delete, rotate, reorder, extract; **O.2** split and merge; **O.3** crop;
  previews and undo.
- P0 **O.22** Print on every desktop.
- P1 **O.4** OCR to searchable PDF; **O.5** edit text runs where the PDF allows; **O.6** replace images; **O.7** true
  redaction (§14.3); **O.8** compress; **O.9** image ↔ PDF; **O.10** PDF → Markdown / DOCX.
- P1 Forms: **O.11** fill, validate, calculate (MuPDF + MuJS); **O.12** save and reuse values; **O.13** flatten on
  request.
- P1 Signatures: **O.14** sign; **O.15** validate, show trust chain and timestamp; **O.16** warn when an edit would
  invalidate a signature, keep signed revisions; never confuse a signature picture with a cryptographic signature.
- P1 **O.23** Screenshots and region snapshots; **O.25** attachments.
- P2 **O.17** Bates numbering; **O.18** headers and footers, watermarks; **O.19** permissions; **O.20** PAdES LTV
  (`/DSS`); **O.21** camera scan on tablets; **O.24** color-accurate and print-production view (Appendix D.4).

### 11.12 Sync and sharing

- P1 **Y.1** Sync through any folder (Syncthing, Dropbox, OneDrive, iCloud) with CRDT merge for notes and
  annotations; no account.
- P1 **Y.2** Same files and settings on every OS.
- P2 **Y.3** Share bundles (PDF + notes + audio); comment threads; **Y.4** live co-editing through a self-hostable
  relay.

### 11.13 Extensibility

- P1 **E.1** CLI for batch jobs (`excellent-cli`).
- P1 Agent interface: **E.2** one JSON-Schema tool table over the engine, used by `excellent-cli run`, **E.3** an
  opt-in MCP server confined to a folder, and **E.4** an opt-in UI control channel for agents and UI tests
  (Appendix C.3).
- P2 **E.5** Plugins as signed TSX packages running in NSH's sandbox, with capabilities the app grants (NSH supports
  this natively).

### 11.14 Bookmarks: viewing and editing

**Two kinds of bookmark, kept apart in the UI and in storage:**

| | Document outline | Personal bookmarks |
| --- | --- | --- |
| What | The PDF's own table of contents (`/Outlines`) | The user's own saved places, in any format |
| Stored | Inside the PDF (incremental save) | `library.db` (synced with the library, §11.12) |
| Seen by | Every PDF reader | ExcellentPDF only |
| Formats | PDF editable; EPUB, XPS, CBZ, etc. read-only | All formats |
| Extras | Title, destination, bold / italic, color, open / closed | Title, destination, note, color, tags, created date |

SumatraPDF Enhanced only *reads* outline styles (bold, italic, color in `EngineMupdf.cpp`); editing is new work.

**Features**

- P0 **B.1** Add an entry at the current view (page, scroll position, zoom) or from selected text (title = the text,
  destination = the top of the selection). If the document has no outline, the first add creates one.
- P0 **B.2** Rename inline (F2); delete (with its children, or promoting them one level).
- P0 **B.3** Move by drag and drop (before, after, or into another entry) and by keyboard: Alt+↑ / Alt+↓ to move,
  Tab / Shift+Tab to indent / outdent.
- P0 **B.4** "Set destination to current view" to repoint an entry; the destination shows as page number and zoom.
- P0 **B.5** Multi-select for delete, move and restyle; undo / redo for every edit; open / closed state saved.
- P0 **B.6** Personal bookmarks: add (Ctrl+D), list, jump, note, color, tag, search; shown in their own sidebar tab.
- P1 **B.7** Bold, italic and color (with the theme-aware drawing Enhanced already uses).
- P1 **B.8** Generate an outline:
  - from headings (font size and weight in structured text, plus Enhanced's numbered-heading rules such as
    "1.2.", "II.A.");
  - from the PDF's structure tags;
  - one entry per file when combining (§11.11).
  Shown as a preview tree to accept, edit or discard.
- P1 **B.9** Check destinations: flag entries pointing to missing pages or nowhere; fix in one click.
- P1 **B.10** Sort children by page; promote personal bookmarks into the document outline (and back).
- P1 **B.11** Import and export: indented text, JSON, and pdftk's bookmark format; copy the outline as Markdown.
- P2 **B.12** Other actions (open a link, open another file, named actions) shown with an icon and editable in the inspector.
- P2 **B.13** Custom outline for EPUB, CBZ and other read-only formats, kept in the sidecar and shown instead of the
  built-in one.

**How it works**

1. **MuPDF API.** `fz_new_outline_iterator` (`pdf_new_outline_iterator` for PDF) walks and edits the real outline:
   - `fz_outline_iterator_next`, `_prev`, `_up` and `_down` move through it;
   - `fz_outline_iterator_insert` adds an entry before the current position;
   - `fz_outline_iterator_update` changes title, destination, flags and color;
   - `fz_outline_iterator_delete` removes an entry and its children.

   Each `fz_outline_item` carries `title`, `uri`, `is_open`, `flags` (bold / italic) and `r g b`. Destinations are
   written as URIs with `fz_format_link_uri` (or `pdf_new_uri_from_explicit_dest`) from an `fz_link_dest` (page,
   `XYZ` / `Fit` / `FitH`..., x, y, zoom) and read back with `fz_resolve_link_dest`. All are in the 1.28.5 headers.
2. **Moves.** The iterator has no move operation, so a move is: read the subtree into memory, delete it, walk to
   the target, insert it again (children re-inserted below). The whole move is one journal operation.
3. **Undo.** Every edit runs inside `pdf_begin_operation` / `pdf_end_operation`, so MuPDF's journal handles undo,
   redo and crash recovery as for annotations (§9.3).
4. **Model.** `excellent-core::outline` keeps a tree of entries with stable ids for the UI. The ids map to iterator
   paths (child indexes from the root). After each edit, the changed branch is re-read from MuPDF. The `/Count`
   and `/First` / `/Last` / `/Prev` / `/Next` links stay MuPDF's job, not ours.
5. **Pages.** When pages are deleted, moved or extracted (§11.11), entries pointing at them are updated or flagged.
   Check what `pdf_rearrange_pages` already does to the outline before writing our own fix-up.
6. **Saving.** Incremental save; encrypted documents stay encrypted; the document's "modify" permission is honoured.
   Editing a signed document warns first, since an outline change may invalidate the signature.
7. **Non-PDF formats.** The outline is read-only. Edits go into personal bookmarks, or into an optional custom
   outline kept in the sidecar file (P2).

**UI**

- Sidebar tab "Outline" uses NSH `Tree` (virtualized). Check that `Tree` supports drag and drop and inline rename;
  where it doesn't, add both in our own wrapper or upstream.
- Edit mode toggles from the outline's toolbar so a plain click on a reader's outline never starts a rename.
- Context menu: Add before / after / child, Rename, Set destination, Style, Delete, Promote / Demote, Sort, Check.
- Inspector: title, destination (page, zoom, position), style, action, children count.
- Command palette: every action above. Hovering an entry previews its destination page.

**Tests:** outlines edited here open correctly in Acrobat, PDF-XChange, Preview, pdf.js and SumatraPDF; `qpdf --check`
passes; a 10,000-entry outline stays smooth; moves deep in the tree keep every child; undo restores the exact bytes
of the previous revision.

### 11.15 Split view

Like VS Code's editor groups, Notepad++'s two views and Vivaldi's tab tiling. **The same file or different files**
can sit side by side in one window.

**Features**

- P0 **V.1** Split the current tab to the right or down. The new pane shows **the same document** at the same place, then
  each pane scrolls, zooms and rotates on its own (Notepad++'s "Clone to other view").
- P0 **V.2** Open **another file** into a pane: from the file dialog ("Open in split"), recent files, the library, or by
  dropping a file on the left / right / top / bottom edge of the page area.
- P0 **V.3** Each pane is a tab group with its own tabs. Drag tabs between groups, or "Move to other group" (Notepad++'s
  "Move to other view").
- P0 **V.4** Two panes side by side or stacked; up to four in a grid (P1 for more). Drag the divider; double-click it to
  make the panes equal.
- P0 **V.5** Active pane: clear focus ring in the mode's accent color. Keyboard focus moves with Ctrl+1 … Ctrl+4. All
  shortcuts and commands act on the active pane.
- P0 **V.6** Close a pane: its tabs move to the neighbouring group (or close, as the user prefers).
- P0 **V.7** Session restore and workspaces (§11.16) keep the split layout, each pane's tabs and each view's position.
- P1 **V.8** Synchronised scrolling, off by default:
  - same page number;
  - same relative position;
  - matched page pairs (for comparing two versions).
  Zoom can sync separately.
- P1 **V.9** **Open a link in the other pane:** Ctrl+click (or middle-click) a link, footnote, citation, outline entry or
  search result. The reading position stays put and the target opens beside it. This is the main reading use for
  papers and textbooks.
- P1 **V.10** Swap panes; flip between side-by-side and stacked; maximise one pane temporarily (double-click its tab).
- P1 **V.11** Pop a tab or group out into its own window (NSH desktop windows), and drag it back.
- P2 **V.12** Vivaldi-style tiling: select several tabs, "Tile" them into a grid; "Untile" returns them to one group.

**How it works**

1. **Document vs view.** `excellent-core` separates the two:
   - a **Document** holds one open file: the MuPDF document, display lists, annotations, the undo journal, the
     dirty flag and the save state;
   - a **View** holds one pane's state: viewport, zoom, rotation, layout mode, selection, find state, history and a
     per-view dark-mode override.

   One Document can have several Views. Tabs and panes hold Views.
2. **Shared work.** Both views of a document share its display lists and tile cache (tiles are keyed by
   document, page and zoom bucket, §7.4). Two panes at the same zoom render a page once, and memory budgets are
   per process, not per pane.
3. **Live edits.** An edit in one view bumps the document's revision for the changed pages. Every view of that
   document re-renders those pages, so an annotation drawn on the left appears on the right in the same frame.
   Undo is per document, so it works the same from either pane.
4. **Saving and closing.**
   - Save, dirty markers and "unsaved changes" are per document; both tabs show the dirty dot.
   - Closing one view of a document never prompts.
   - Closing its last view prompts if it is unsaved.
5. **Panes and tabs.** The layout is NSH `DockArea` (groups, splits, drag-to-dock) or `ResizablePanelGroup` plus
   `Tabs`. Its layout is plain data, so sessions and workspaces store it.
6. **Canvases.** Each visible pane has its own `excellent.page-canvas` instance. On Windows that is one child window
   and swap chain per pane. Hidden tabs release their surfaces and keep only their View state.
7. **Pen and input.** Pen and touch go to the pane under the pen. Palm rejection runs per pane.
8. **Tablets (V.14).** iPad and Android tablets in landscape get two panes, plus the "open link in other pane" behaviour.
   Phones stay single-pane.

**Commands and shortcuts** (all in the command palette and the tab context menu):

| Action | Shortcut |
| --- | --- |
| Split right (same document) | Ctrl+\ |
| Split down (same document) | Ctrl+Alt+\ |
| Open file in other pane | Ctrl+Shift+O |
| Move tab to next / previous group | Ctrl+Alt+→ / Ctrl+Alt+← |
| Focus pane 1–4 | Ctrl+1 … Ctrl+4 |
| Toggle synchronised scrolling | Ctrl+Alt+S |
| Swap panes | (palette and menu) |

Command line (**V.13**): `excellentpdf --split a.pdf b.pdf` and `excellentpdf --split-same a.pdf`.

**Budgets:** two panes scrolling at once still meet 60 / 120 fps (§13). Opening a split of an already-open document
reuses its Document and costs no re-parse; the target is ≤ 50 ms to the second pane's first frame.

**Tests:**
- Edits in one pane show in the other at once.
- Undo from either pane works.
- Closing panes in any order never loses unsaved work.
- Sessions restore exactly.
- Synced scrolling stays aligned across documents with different page sizes.

### 11.16 Workspaces

Like Vivaldi's workspaces: named sets of tabs, so "Thesis", "Course: Physics 201" and "Tax 2026" each keep their
own files. Switching swaps the whole tab strip and layout in the current window. Only one workspace is visible per
window, and the others cost almost nothing until opened.

**What a workspace holds**

- Its tabs and tab groups, pinned tabs, and the split layout (§11.15).
- Each tab's View state: page, scroll position, zoom, rotation, layout mode and dark-mode override.
- The active tab, sidebar tab and panel sizes.
- Recently closed tabs, so Ctrl+Shift+T reopens within the workspace.
- A name, an icon (emoji or symbol) and a color. The color tints the workspace button and tab strip, using the
  theme's accent rules (§10.3).

**Features**

- P0 **W.1** **Switcher** at the left end of the tab bar, showing the current workspace's icon and name. Click it for a
  popover listing every workspace with its color, tab count and unsaved-changes dot. It is also in the left rail
  and the command palette ("Switch to workspace…").
- P0 **W.2** Create a workspace: empty, from all current tabs, or from selected tabs (multi-select, then "Move to new
  workspace").
- P0 **W.3** Rename, recolor, change icon, reorder by drag, delete. Deleting asks whether to close its tabs or move them
  to another workspace.
- P0 **W.4** Move or copy a tab to another workspace: tab context menu, or drag the tab onto the switcher.
- P0 **W.5** Shortcuts: Ctrl+Alt+1 … 9 for the first nine workspaces; Ctrl+Alt+PgUp / PgDn for previous / next.
- P0 **W.6** Opening a file adds it to the current workspace. If the file is already open in another workspace, a setting
  chooses: switch there (default) or open here as well.
- P0 **W.7** Session restore reopens each window on its last workspace.
- P1 **W.8** A workspace per window: "Open workspace in new window"; two windows can never show the same workspace.
- P1 **W.9** Search across workspaces: the tab search (Ctrl+Shift+A) lists tabs from every workspace, grouped, and jumps
  to the right one.
- P1 **W.10** Link to the library: "Open collection as workspace", and "Save workspace as collection".
- P1 **W.11** Export and import a workspace as a small JSON file (paths stored relative to a chosen root where possible).
  Files that are missing on import show greyed out with "Locate…".
- P2 **W.12** Templates (e.g. "Course": syllabus, textbook, notebook) and rules ("files from this folder open in workspace
  X").
- P2 **W.13** Workspaces sync between devices through folder sync (§11.12).

**How it stays efficient**

1. **Inactive tabs are data, not documents.** A tab in an inactive workspace is a record of about 1 KB: file
   identity (path, size, modified time, and a hash of the first and last 64 KB to find moved files), title, and
   its View state. No MuPDF document, no file handle, no display lists, no tiles. A hundred workspaces of fifty
   tabs each cost a few megabytes of SQLite rows.
2. **Lazy restore on switch (W.14).** On a switch:
   - save the outgoing View states (already kept current, so this is a write of dirty rows only);
   - hide the outgoing panes' canvases;
   - open **only the visible tab of each pane** in the incoming workspace;
   - load other tabs when they are first clicked, with a page thumbnail shown meanwhile. Background tabs may load
     at idle priority (§7.4), stopping on memory pressure.
3. **A warm cache for quick back-and-forth.** The documents of the last workspace or two stay open (warm) within a
   memory budget, so switching back is instant. Least recently used workspaces are hibernated first: their
   display lists and tiles are dropped, then the MuPDF documents are closed.
4. **One Document per file.** A document registry, keyed by file identity and reference-counted, means a file open
   in two workspaces is parsed once and shares caches, annotations and undo (as split view does, §11.15).
5. **Unsaved work is never hibernated.** A document with unsaved changes stays open, or its journal is flushed
   first (§9.3). The workspace shows a dirty dot, and quitting lists unsaved files from every workspace.
6. **No file locks.** Hibernated and inactive tabs hold no file handles, so on Windows other programs can save,
   rename or delete those files. Changes are picked up when the tab is opened again. A missing file shows a clear
   message with "Locate…".
7. **Thumbnails.** One small page thumbnail per tab (WebP, about 10 KB) is cached on disk for the switcher and for
   tabs not yet loaded.
8. **Storage.** Three tables in `library.db`:
   - `workspace`: id, name, icon, color, order, layout JSON, active tab;
   - `workspace_tab`: id, workspace, group, order, file id, view state, pinned;
   - `closed_tab`: per workspace, capped at 50.

   Changes are written in a debounced transaction about 1 s after the last change, and at once on switch and on
   quit. Crash recovery restores the last written state.

**Budgets** (added to §13):

| Metric | Budget |
| --- | --- |
| Switch to a warm workspace, first frame | ≤ 50 ms |
| Switch to a cold workspace, first page painted | ≤ 150 ms (the open-document budget) |
| Memory per inactive tab | ≤ 2 KB plus its thumbnail on disk |
| Startup with 100 workspaces / 5,000 tabs | no slower than with one |

**Tests:**
- Switching back and forth 1,000 times leaks no memory.
- Hibernation never drops unsaved changes.
- A file moved or renamed while inactive is found again by its hash.
- Two windows never show the same workspace.
- Session restore and import / export round-trip exactly.

---

## 12. AI, search and knowledge

### 12.1 Capabilities

- **I.1** Providers: local packs and cloud providers, off until the user turns one on.
- **I.2** Document index for AI (chunks with page anchors).
- **I.3** Ask about the selection, page, document or library with page-cited answers (the AI chat panel).
- **I.4** Summaries; **I.5** plain-language rewrites and explanations.
- **I.6** Explain equations step by step.
- **I.7** Define terms in context.
- **I.8** Glossary, study guide and flashcard generation.
- **I.9** Claim, method, finding and limitation extraction.
- **I.10** Cross-document comparison and evidence tables.
- **I.11** Suggested highlights and tags (never applied silently).
- **I.12** Evaluation and safety suite for all of the above.

### 12.2 Pipeline

1. **Chunking:** paragraph windows (about 512 tokens, overlapping) from structured text, each with page number and
   quads.
2. **Embeddings:** local ONNX model (e.g. BGE-small, int8) as a pack; index in HNSW (`usearch`).
3. **Retrieval:** hybrid FTS5 + vector + metadata filters.
4. **Generation:** local model through llama.cpp or Ollama (pack), or a cloud provider the user picks.
5. **Citations:** every factual statement carries `{ page, quads, verbatim quote }`; clicking `[1]` scrolls the canvas
   there and pulses a highlight.

### 12.3 Rules

- Never invent page numbers or citations; say so when evidence is missing.
- Document text is untrusted: instructions inside a PDF are never followed (prompt-injection defense).
- No document leaves the device without explicit consent; a visible local / cloud indicator.
- AI can be turned off completely, per document and per workspace.
- Destructive or visible actions suggested by AI need confirmation.
- An evaluation suite measures citation accuracy and hallucination rate, including adversarial documents.

---

## 13. Performance budgets and instrumentation

### 13.1 Budgets (CI gates, on a defined reference machine)

| Metric | Budget | Stretch |
| --- | --- | --- |
| Cold start to usable window | ≤ 300 ms | ≤ 150 ms |
| Open a typical PDF to first page painted | ≤ 150 ms | ≤ 60 ms |
| Open a 10,000-page PDF to first page | ≤ 400 ms | |
| Scroll and page turn | 60 fps, 120 on 120 Hz displays | |
| Ink: pen-down to pixels | ≤ 1 display frame | |
| Search 1,000 pages (indexed) | ≤ 100 ms | |
| Idle memory, one small document | ≤ 60 MB | ≤ 30 MB |
| 20 open tabs | ≤ 500 MB | |
| UI-thread stall | never > 50 ms | |
| Core installer | ≤ 15 MB | |

A pull request that regresses a budget by more than 10 % fails CI. Budgets live in `tests/perf/budgets.json` and
`tests/size/budgets.json`. The blueprint's figures (< 35 ms start, < 28 MB idle) are kept as stretch aims only; NSH's
own startup and MuPDF's font loading must be measured first (Phase 0).

### 13.2 What we measure

Cold and warm start; time to first page and to first interactive scroll; frame-time distribution during scroll and
zoom; render latency by page complexity; selection latency; annotation display and save latency; search latency and
index freshness; OCR throughput and cancel time; memory and cache hit rates; idle CPU and GPU; battery on tablets;
AI time to first token and citation coverage. Logs go through the `log` facade with a `perf` target. Telemetry, if
any, is opt-in and never contains document content.

---

## 14. Reliability, security and privacy

### 14.1 Reliability

Journaled autosave; atomic saves; recovery replay; undo / redo as transactions; backups with restore tests; clear
errors with recovery actions; progress and cancel for every long job; never silent data loss.

### 14.2 Security

- Treat every document as hostile. Limits on image size, object counts, recursion and decompression.
- Isolate parsing: render workers can be moved to a separate process later if fuzzing shows risk; start with
  in-process plus fuzzing.
- Never auto-run attachments, scripts or links; confirm external links; block path traversal in attachments.
- Form JavaScript runs in MuJS with MuPDF's restricted API only.
- Credentials and AI keys in the OS key store (Credential Manager, Keychain, libsecret).
- Packs and plugins are signed (Ed25519) and verified before loading.
- Dependency scanning (`cargo deny`, `cargo audit`); fuzzing in CI; a security disclosure process.

### 14.3 True redaction

1. Mark areas or search-and-mark patterns (names, numbers, regex); preview.
2. Apply with `pdf_apply_redaction` (text, images, line art) per marked annotation.
3. Scrub: document info, XMP metadata, attachments, hidden layers, thumbnails, comments, form values in the area.
4. Save as a **new full file** (not incremental) with garbage collection, so earlier revisions are gone.
5. **Verify:** reopen the output and search for the removed text and patterns; show a report. Never claim complete
   removal without this check.

### 14.4 Privacy

Local-first; no account for offline features; explicit consent for cloud AI; retention and deletion controls;
encrypted sync where supported; separate private notes from shared comments.

---

## 15. Accessibility

- **U.1** Full keyboard operation with logical focus order and visible focus.
- NSH gives accessible chrome on every platform (UI Automation, AT-SPI, NSAccessibility, iOS, Android).
- **The canvas must expose its own accessibility tree:** pages, text in reading order (from structured text and the
  PDF's tags), links, form fields and annotations. On Windows this means a UI Automation provider on the canvas window,
  including the Text pattern so screen readers can read and select document text (**U.2**; other platforms **U.3**).
- **U.4** Tagged-PDF reading order; alt text; high-contrast and color-blind-safe themes; UI scaling; reduced motion.
- **U.5** Read aloud with sentence and word highlighting (SAPI / OneCore, AVSpeechSynthesizer, speech-dispatcher,
  Android TTS); **U.6** neural offline voices as packs.
- **U.7** Localization (Appendix D.4); **U.8** input methods (IME) on the canvas (Appendix D.1); **U.9** first-run
  guide and tips.
- Tested with real assistive technology (Narrator, NVDA, VoiceOver, Orca, TalkBack).

---

## 16. Testing and quality

| Area | How |
| --- | --- |
| Models and core | `cargo test`; property tests for coordinate transforms and layouts |
| MuPDF layer | Golden-image rendering against a corpus; compare with Enhanced's output on the same files |
| UI | `nsh test` headless runs and per-platform runs (`--target windows`, `macos`, `linux`, ...) |
| Visual regression | Screenshots per theme and mode; diffs reviewed |
| Annotations | Round-trip: create here → open in Acrobat, PDF-XChange, Preview, pdf.js, Sumatra → compare; geometry under rotation and crop boxes |
| Ink | Recorded pen sessions replayed; latency measured with a high-speed camera on each platform |
| Redaction | Automated verification on a corpus with known secrets |
| Signatures | Validation against reference signed files; invalidation warnings |
| Fuzzing | `cargo fuzz` on open / render / text / save through the safe API |
| Performance | Benchmarks on small, medium, large, scanned, vector-heavy and image-heavy files; budgets in CI |
| Recovery | Kill the process during saves and edits; check nothing is lost |
| Accessibility | Automated checks plus manual screen-reader passes |
| Usability | Each release: five new users complete core tasks without help |

**Corpus:** public and synthetic PDFs (papers, books, scans, forms, CAD drawings, huge manuals, malformed files), plus
Sumatra's bug repro files where licences allow, the pdf.js test corpus, and CJK and right-to-left documents.
Every saved file passes `qpdf --check`; `pdftotext` and `pdfsig` serve as text and signature oracles. Saved files
are opened in other readers (interop suite) before any public release (Appendix C.4).

---

## 17. Repository layout and engineering rules

```
ExcellentPDF/
├─ README.md  CHANGELOG.md  LICENSE (AGPL-3.0, see §4)  AGENTS.md
├─ Cargo.toml  Cargo.lock  rust-toolchain.toml  deny.toml
├─ .github/workflows/      build (reusable); windows; macos; linux; android; ios; budgets; fuzz; release
├─ crates/                 see §5.4
├─ third_party/
│  ├─ mupdf/               Sumatra's 1.28.5 tree
│  ├─ mupdf-patches/       one .patch per change, with README (Sumatra's process)
│  ├─ amalgamations/       freetype, harfbuzz, lcms2, tesseract, zxing-cpp, ... (from Sumatra's ext/)
│  └─ sumatra/             Sumatra's own MuPDF glue (src/shared/mupdf, src/ng/mupdf, src/ng/libarchive),
│                          kept out of mupdf/
├─ app/                    nsh.json, assets (one UI font, SVG icons, themes)
├─ android/                Java/Kotlin canvas view and app shell
├─ packs/                  sources for optional downloads (never in the core installer)
│  fonts/ dictionaries/ handwriting/ ocr/ ai-models/ voices/ templates/ stickers/
├─ design/                 mockups, palettes, icon sources, brand
├─ docs/
│  ├─ dev-notes/           plans (this file and its sources)
│  ├─ adr/                 decision records (one per §3 row to start)
│  ├─ features/            one spec per feature, from _template.md
│  └─ user-guide/
├─ tests/
│  corpus/  golden/  perf/budgets.json  size/budgets.json  fuzz/
└─ packaging/              windows/ (installer, portable zip)  macos/ (dmg, notarize)  linux/ (AppImage, Flatpak, deb)
```

**Rules**

- A feature is a self-contained module: model, view, tests. No OS types in models; features don't reach into each
  other's internals.
- Budgets are files, not promises.
- Packs live outside the core.
- Workspace lints: `missing_docs = warn`, `unreachable_pub = deny`; `cargo clippy` clean; `cargo +nightly fmt` if we
  adopt NSH's formatting options.
- Dependencies declared once in the root `[workspace.dependencies]`.
- Pin NSH to an exact release from crates.io (`=0.5.0` today; its API isn't frozen) and update deliberately, in a
  pull request of its own.
- `AGENTS.md` (with `CLAUDE.md` pointing to it) holds these rules for AI agents; keep it in step with this plan.
- Every phase ends with a tagged build on all supported platforms and a short acceptance checklist.

---

## 18. Phased roadmap

Estimates assume one developer working with AI coding agents; they are rough.

### Phase 0 — Spikes and foundations (≈ 3–4 weeks)

Prove the risky parts before building features.

- [ ] Vendor Sumatra's MuPDF and patches; `excellent-mupdf-sys` builds with `cc` on Windows (MSVC); C shims for open,
      page count, render to pixmap, structured text.
- [ ] Safe API: open a PDF, render a page to PNG, extract text; leak-free under ASan; first fuzz target.
- [ ] Threading: four workers rendering display lists of one document at once; no crashes over 10,000 renders.
- [ ] NSH app skeleton (`nsh init --template rust`): window, `DockArea` with sidebar and inspector placeholders,
      command palette, keymap; measure NSH cold start.
- [ ] Canvas spike on Windows: child-window component with a swap chain showing MuPDF tiles; scroll and zoom one
      document smoothly.
- [ ] Pen spike on Windows: `WM_POINTER` pressure ink drawn in the same frame; measure latency.
- [ ] CI: Windows build, tests, size and startup budgets, `cargo deny`.
- [ ] ADRs for D1–D10; licence decision (Q1).

**Done when:** a document scrolls at 60 fps in the NSH window with a pressure stroke drawn on it, and the measured
start, first-page and ink numbers are written down.

*Status 2026-11-13:* T1 to T10 are built. Start and first-page numbers are written down (K.24); both are over budget,
mostly in NSH's startup. Open: the ink number and the pressure stroke over a scrolling document need a pen, and the
maintainer tests them on a Surface Pro; 60 fps is not measured yet (K.24's harness); ASan and the fuzz target run in
CI only, and CI has not run, as nothing is pushed; the licence (Q1) is open. The maintainer asked on 2026-11-13 to go
on to Phase 1 while these close.

### Phase 1 — Reader MVP on Windows (≈ 6–8 weeks)

- [ ] Layouts, zoom, rotation, thumbnails, outline, bookmarks, links, history, tabs, session restore.
- [ ] Document / View split in the core; split view for the same and different files, tab groups (§11.15 P0).
- [ ] Outline editing and personal bookmarks (§11.14 P0).
- [ ] Document registry (one Document per file, reference-counted, hibernation); workspaces with switcher and lazy
      restore (§11.16 P0).
- [ ] Text selection and copy; find in document; smart dark mode ported from Enhanced; themes.
- [ ] Library basics: recent files, pinned files, drag and drop, file associations, single instance.
- [ ] Settings, keyboard shortcuts, command palette covering every action.
- [ ] Accessibility: keyboard everywhere; first UI Automation tree for the canvas.
- [ ] Installer and portable zip under the size budget.

**Done when:** the Rust app opens everything Enhanced opens via MuPDF, faster or equal on the corpus, and can be used
daily for reading.

### Phase 2 — macOS and Linux (≈ 4–6 weeks; can overlap Phase 3)

- [ ] Canvas components for macOS (Metal) and Linux (GTK 4); pen input on both.
- [ ] Platform services: dialogs, print, clipboard, key store, TTS, shell integration.
- [ ] CI matrix on all three desktops; packaging (signed and notarized DMG; AppImage first, then Flatpak and deb).

**Done when:** Phase 1's checklist passes on all three desktops.

### Phase 3 — Annotation and ink (≈ 8–10 weeks)

- [ ] Markup, notes, shapes, stamps, inspector, filters; undo / redo through MuPDF's journal; autosave and recovery.
- [ ] Ink engine (§8): pens, erasers, lasso, presets, palm rejection; pressure appearance streams.
- [ ] Round-trip tests in other readers.

**Done when:** annotation and ink parity with Enhanced, with nothing lost across crash tests.

### Phase 4 — Notebooks, audio and study (≈ 8–10 weeks)

- [ ] Notebooks with procedural templates, page insertion, the `.expdf` format.
- [ ] Audio recording synced to ink.
- [ ] Flashcards and FSRS; vocabulary and dictionary ported from Enhanced (dictionaries as packs).
- [ ] Present mode and presenter view.

### Phase 5 — Search, library and knowledge (≈ 6–8 weeks)

- [ ] SQLite library, FTS5 index over documents, annotations and notes; search operators; collections and tags.
- [ ] OCR pack (Tesseract through MuPDF); handwriting search.
- [ ] Notes with backlinks; citations; Markdown export.
- [ ] Pack manager (download, verify, remove, offline).

### Phase 6 — AI (≈ 4–6 weeks)

- [ ] Provider trait, local pack, cloud adapters with consent; RAG with clickable citations; evaluation suite.

### Phase 7 — Professional tools (≈ 8–10 weeks)

- [ ] Page organizer; forms with calculation; signatures (Rust signer replacing patch `0003`, OS key stores); true
      redaction with verification; compression; conversions; comparison.

### Phase 8 — Tablets (after desktop parity)

- [ ] iPad canvas (UIKit, Apple Pencil) and Android canvas (Java/Kotlin + JNI, stylus); touch-first layouts using NSH's
      mobile presentation (sheets, tab bar).
- [ ] Camera scan; palm rejection tuned per device.

### Phase 9 — Ecosystem

- [ ] Folder sync with CRDTs; sharing bundles; teaching tools (grading, classroom); research canvas; plugins; web
      viewer (WASM, read-only first, AGPL source offer).

### Timeline sketch

| Weeks | Milestone |
| --- | --- |
| 1–4 | M0: spikes pass, numbers measured, CI green |
| 5–12 | M1: Windows reader MVP ("alpha") |
| 13–18 | M2: macOS + Linux readers |
| 13–22 | M3: annotation and ink ("beta"; replaces Enhanced for daily use) |
| 23–32 | M4: notebooks, audio, study, present |
| 33–40 | M5: search, library, OCR, packs |
| 41–46 | M6: AI |
| 47–56 | M7: professional tools ("1.0") |
| later | M8 tablets, M9 ecosystem |

---

## 19. What was dropped from the source documents, and why

| Dropped | Source | Reason |
| --- | --- | --- |
| Tauri v2 + React / SolidJS + Tailwind UI | `Earths_num1_pdf_application.md` | Replaced by NSH (native views, no web runtime) |
| Google PDFium | `Earths_num1_pdf_application.md` | Replaced by MuPDF to match SumatraPDF |
| Custom parser, XRef reconstructor, mmap I/O layer | `Earths_num1_pdf_application.md` | MuPDF does this; its stream layer reads lazily |
| WGPU compositor and the WGSL luminance shader | `Earths_num1_pdf_application.md` | Each platform presents natively; Enhanced's object-level dark mode is better than a per-pixel shader |
| Custom `/AP` generator and incremental writer | `Earths_num1_pdf_application.md` | MuPDF provides both; the sample writer also miscalculated `/Size` |
| QuickJS for form scripts | `Earths_num1_pdf_application.md` | MuPDF's MuJS integration |
| "< 4.2 ms" pen latency and "< 35 ms" start as hard targets | `Earths_num1_pdf_application.md` | The first is below one display refresh; both become stretch aims after measurement |
| Open-core / commercial licence | `Earths_num1_pdf_application.md` | Not possible with AGPL MuPDF unless a commercial licence is bought (Q1) |
| Restoring Sumatra's GTK / Cocoa ports, `VirtCtrl` shared UI, `cmd/build.ts` | `ExcellentPDF-cross-platform-plan.md` | C++ fork approach replaced by the Rust rewrite; its phase discipline, service table and risk handling are kept |
| Keeping upstream SumatraPDF history in this repo | `layout and design.md` | Only needed for a fork; we vendor MuPDF and its patches instead |
| The fictional `github.com/excellent-pdf/aether-pdf` clone URL | `Earths_num1_pdf_application.md` | The repo is `github.com/abelokoj/ExcellentPDF` |

---

## 20. Risks

| Risk | Mitigation |
| --- | --- |
| NSH is young (v0.5.0, API not frozen; about 670 commits in the week after 0.4.0) | Pin a tag; keep NSH-facing code in the app and view layers; re-check upstream before each phase; upstream fixes we need |
| NSH gives apps no pen pressure or tilt; the Windows child-window canvas can't sit under NSH overlays | Read the pen from the OS in the canvas, as Enhanced does (§7.3); draw overlays inside the canvas; propose pen events upstream (T9) |
| MuPDF build on five platforms from Rust | Start with Windows in Phase 0; follow mupdf-rs; amalgamated thirdparty |
| `longjmp` across Rust frames | C shims around every call; enforced by keeping `excellent-mupdf-sys` the only crate that touches raw MuPDF |
| MuPDF thread safety | Locks context, per-thread cloned contexts, document mutex, Sumatra patch 0007; stress test in Phase 0 |
| Staying in sync with Sumatra's MuPDF patches | Same patch process; review upstream SumatraPDF's `ext/patches` at each MuPDF update. Enhanced and upstream reuse patch numbers, so ours are renumbered and the README records each patch's origin |
| Ink latency on GTK / Wayland and Android | Platform-native low-latency paths; prediction; measure per device |
| Canvas accessibility is a large job | Plan the UIA / AT-SPI / NSAccessibility providers from Phase 1, not at the end |
| Scope: the feature list is enormous | Phase gates; P0 before P1; parity with Enhanced before new features |
| AGPL limits business options | Decide Q1 early; a commercial MuPDF licence is the only exit |
| Hostile documents | Limits, fuzzing, later process isolation |
| Mac hardware and Apple developer account | GitHub macOS runners for builds; a real Mac for QA; $99/year account before notarizing |

---

## 21. Decisions needed from the maintainer

- **Q1 Licence:** ship AGPL-3.0 (recommended, free), or buy a commercial MuPDF licence to keep closed or dual-licensed
  options open.
- **Q2 Enhanced's future:** freeze Enhanced to fixes once the Rust app reaches M3 (recommended), or keep adding
  features to both.
- **Q3 Pen events in NSH:** contribute pen-aware pointer events upstream (recommended), or keep all pen handling in our
  canvas only.
- **Q4 Linux packaging order:** AppImage first (recommended) or Flatpak first.
- **Q5 Non-MuPDF formats:** bring DjVu, CHM and CBR to the Rust app in Phase 5 (recommended), or drop them.
- **Q6 Mac hardware:** availability of an Apple Silicon Mac and an Apple developer account by Phase 2.
- **Q7 Name and branding:** "ExcellentPDF" in binaries, installer IDs and icons; side-by-side install with
  SumatraPDF and Enhanced.

---

## 22. First ten tasks

1. **T1** Create the Cargo workspace and the folder layout of §17; add `AGENTS.md` with the rules of §17.
2. **T2** Copy upstream SumatraPDF's `ext/mupdf`, `ext/patches` (70 patches), `src/shared/mupdf` and the needed
   `ext/a-*` amalgamations into `third_party/`; add Enhanced's six annotation patches as `0088`–`0093` (§6.1); record
   each source revision.
3. **T3** `excellent-mupdf-sys`: `build.rs` compiling all of MuPDF for every target OS, tested on Windows (MSVC)
   (`dev_decisions.md` DD1); C shims for open, count pages, load page, render to pixmap, structured text; generated
   bindings committed.
4. **T4** `excellent-mupdf`: safe types with `Drop`; a CLI test that renders page 1 of a corpus file to PNG.
5. **T5** Threading stress test: four workers rendering display lists of one document, run under ASan.
6. **T6** NSH app skeleton with `DockArea`, a sidebar, an inspector, the command palette and keymap; measure cold
   start.
7. **T7** `excellent.page-canvas` on Windows as a child window with a swap chain, showing tiles from the core, with
   scroll and wheel zoom.
8. **T8** Windows pen spike: `WM_POINTER` with `GetPointerPenInfo`, pressure-width stroke drawn in the same frame;
   latency measured.
9. **T9** Write the NSH upstream proposal for pen pressure, tilt, rotation and eraser state on pointer events (v0.5.0
   already has `PointerType::Pen`) and pointer input on drawn components.
   *Drafted 2026-11-13:* `dev notes/nsh_upstream_proposal_pen_input.md` (pen state and coalesced samples, pointer
   input for drawn components, and native components layered in NSH's composition tree like web views, which fixes
   overlays). Not filed: the maintainer decides when.
10. **T10** GitHub Actions on Windows: build, tests, `cargo deny`, size and startup budgets. The build part and the
    by-hand workflows for the other platforms are already in place (`dev_decisions.md` DD2); T10 adds the budgets.
    *Done 2026-11-13:* `[profile.release]` (fat LTO, one codegen unit, stripped; panics still unwind, because
    nsh-core catches a component's panic), `tests/budgets.ps1` and the `budgets` job in `windows.yml`. Results in K.24.

---

## 23. Appendix A: feature → MuPDF API map

All functions below exist in the 1.28.5 headers in `sumatrapdf/ext/mupdf/include/mupdf/`.

| Feature | Functions |
| --- | --- |
| Context and threads | `fz_new_context` (with `fz_locks_context`), `fz_clone_context` |
| Open | `fz_open_document` |
| Render | `fz_new_display_list_from_page`, `fz_run_display_list` (scissor rect, `fz_cookie` with `abort` / `progress`) |
| Text and search | `fz_new_stext_page_from_page`, `fz_search_stext_page` |
| Annotations | `pdf_update_annot`, `pdf_add_annot_ink_list`, `pdf_add_annot_ink_list_stroke`, `pdf_annot_ink_list_stroke_vertex`, `pdf_set_annot_border_width`, `pdf_set_annot_appearance`, `pdf_set_annot_appearance_from_display_list` |
| Undo / redo | `pdf_enable_journal`, `pdf_begin_operation`, `pdf_undo`, `pdf_redo` |
| Save | `pdf_save_document` (`pdf_write_options.do_incremental`) |
| Redaction | `pdf_apply_redaction` (`pdf_redact_options`: `black_boxes`, `image_method`, `line_art`, `text`) |
| Forms | `pdf_enable_js` and the form API in `pdf/form.h` |
| Signatures | `pdf_sign_signature`, `pdf_count_signatures` |
| OCR | `fz_new_ocr_device`, `fz_new_pdfocr_writer` |
| Export | `fz_new_document_writer` |
| Outline (bookmarks) | `fz_new_outline_iterator` / `pdf_new_outline_iterator`, `fz_outline_iterator_next` / `_prev` / `_up` / `_down` / `_insert` / `_update` / `_delete`, `fz_format_link_uri`, `pdf_new_uri_from_explicit_dest`, `fz_resolve_link_dest` |
| Page reordering | `pdf_rearrange_pages` |

---

## 24. Appendix B: references

**In this workspace**

- SumatraPDF's MuPDF and patches: `C:\dev\sumatrapdf\ext\mupdf`, `ext\patches\README.md`, `ext\versions.txt`
  (Enhanced); upstream's with `git show upstream/master:ext/patches/README.md` there. Findings: `research.md` R1
- Enhanced's smart dark mode: `C:\dev\sumatrapdf\src\PdfDarkMode.h` and `PdfDarkMode*.cpp`
- Enhanced's feature history: `C:\dev\sumatrapdf\SumatraPDF_Enhanced_CHANGELOG.md`, `SumatraPDF_Enhanced_TODO.md`
- NSH: installed CLI and runtime `C:\dev\nsh` (`nsh 0.5.0`); source `C:\dev\nsh-src` (tag v0.5.0, <https://github.com/longbridge/nsh/blob/v0.5.0>) (`AGENTS.md`, `docs/dev/`,
  `website/docs/native-components.md`,
  `docs/dev/windows.md`)

**External**

- NSH documentation: <https://nsh.dev>
- MuPDF: <https://mupdf.com>, <https://github.com/ArtifexSoftware/mupdf>
- mupdf-rs (reference bindings): <https://github.com/messense/mupdf-rs>
- PDF Association technical resources: <https://pdfa.org/resource/>
- ISO 32000-2 (PDF 2.0), ISO 19005 (PDF/A), ISO 14289 (PDF/UA), ETSI EN 319 142 (PAdES)
- WCAG: <https://www.w3.org/WAI/standards-guidelines/wcag/>
- SQLite FTS5: <https://www.sqlite.org/fts5.html>
- FSRS spaced repetition: <https://github.com/open-spaced-repetition>
- Tesseract: <https://github.com/tesseract-ocr/tesseract>
- llama.cpp: <https://github.com/ggerganov/llama.cpp>
- usearch: <https://github.com/unum-cloud/usearch>
- OWASP ASVS: <https://owasp.org/www-project-application-security-verification-standard/>

---

## 25. Appendix C: lessons from PdfCraft

[PdfCraft](https://github.com/storytold/pdfcraft) (storytold, version 0.18.1, MIT OR Apache-2.0) is a PDF editor
written in Rust from scratch, aimed at matching Acrobat Pro. It uses its own object layer, `lopdf`, and the `hayro`
renderer, with an egui interface. It runs on macOS, Windows, Linux, FreeBSD and the web. By its own assessment
(October 2026) it is **alpha**, about 50 % "ready for real work" against Acrobat Pro, with 94 % of its must-have
features in place. This appendix was written from its README and its parity documents, not from running it.

### C.1 Where it overlaps with this plan

| Area | PdfCraft today | ExcellentPDF |
| --- | --- | --- |
| Comments and markup | Every markup type, replies, status, stamps (custom, dynamic), XFDF / FDF, summaries, flatten, 2D measuring | Same scope (§11.3); match it as the bar for pro annotation |
| Forms | Every AcroForm field type, Acrobat's AF functions, sandboxed JavaScript (`boa`), partial XFA with FormCalc | MuPDF forms + MuJS (§11.11); XFA is a gap for us |
| Redaction | Glyphs, pixels, vectors, annotations and tags removed, then verified; word lists, patterns, sanitize | Same pipeline (§14.3) |
| Signatures | PAdES B-B / B-T, partial LTV, PKCS #12, Windows store and Keychain keys | Rust signer replacing patch `0003` (§6.2) |
| Pages | Organize, combine, split, extract with links, fields, layers, attachments kept | §11.11 |
| Saving | Incremental, atomic, undo after save, autosave, crash recovery | §9.3 |
| Automation | CLI, MCP server, UI control channel, 141 tools | Adopt (C.3) |

### C.2 What it does not cover (where ExcellentPDF is different)

- **Pen:** ink from the pointer only, no pressure or tilt (their gap 20). Pen-first ink is core for us (§8).
- **Formats:** PDF only. MuPDF gives us EPUB, XPS, CBZ, FB2, MOBI, SVG and images, and Sumatra's libraries add DjVu,
  CHM and CBR.
- **Students, teachers, researchers:** no notebooks, audio, flashcards, spaced repetition, presentation tools,
  grading, research canvas or AI (§11.5–§11.10, §12).
- **Reading:** no reflow, split view, multiple windows or read aloud; no smart page recoloring for dark mode.
- **Rendering:** borrowed from the young `hayro` crate (with 31 local patches), fidelity not yet measured. MuPDF is
  the mature engine SumatraPDF already proves.
- **OCR:** Latin script without accents only.
- **Accessibility, touch and tablets:** screen-reader support is partial; no touch mode or tablet apps.

### C.3 Ideas to adopt

1. **Agent interface.** One table of JSON-Schema tools over the engine (open, render, find text, comment, fill
   forms, organize, save), reached three ways: `excellent-cli run`, an opt-in MCP server over stdin / stdout confined
   to a `--root` folder, and a Rust API. An opt-in UI control channel on loopback, with a random token in a file only
   the user can read, lets agents and UI tests drive the real app. Add this to Phase 5 (§11.13).
2. **A feature checklist that CI checks.** A TOML file of every tracked feature with status and the test that proves
   it; CI rejects a "shipped" claim whose command or test doesn't exist. Use it for the Enhanced parity list (§11.1)
   and for pro-tool parity. PdfCraft's Acrobat checklist (about 830 features) is a good starting list for Phase 7.
3. **Honest readiness reports** per area (breadth, depth, hours remaining), updated each milestone.
4. **Corpus and oracles in CI** (§16): the pdf.js test corpus (983 files) for open / render / edit / save sweeps;
   `qpdf --check` on every saved file; `pdftotext` and `pdfsig` as text and signature oracles; nightly fuzzing with a
   regression test per crash.
5. **Render each page in isolation**, so one bad page can't take down the document or the app.
6. **Windows packaging details:** per-user MSI by default, optional context-menu entries ("Combine files in…"),
   a portable zip that keeps settings beside the exe, and signed binaries for every architecture including ARM64.

### C.4 Mistakes to avoid (from their first public releases)

Their 0.4.0 and 0.5.0 releases drew 245 open issues in two days. The largest groups were:

- installing, launching and GPU / window problems (54);
- rendering (46);
- editing text and images (41);
- CJK fonts and glyphs (41);
- signing and certificates (35);
- printing (23).

Encrypted files they wrote didn't open in Acrobat, Windows printing arrived late, and Word export lost layout. They
also have no performance budgets yet. For us this means:

- **Before the first public build:** an interop suite that opens our saved files (encrypted ones included) in
  Acrobat, PDF-XChange, Preview, pdf.js and SumatraPDF; printing on every desktop; and CJK and right-to-left
  documents in the test corpus.
- **Startup:** a fallback path when the GPU or the swap chain fails to start. A software blit in the canvas is
  enough.
- **Budgets:** keep the performance budgets of §13 in CI from Phase 0, as already planned.

### C.5 Reusing its code

- **The licence allows it.** MIT or Apache-2.0 code may be combined into an AGPL program. Keep the copyright notices,
  and carry Apache-2.0's `NOTICE` text, in our `NOTICE` / `ATTRIBUTION` files.
- **The trademark doesn't come with it.** The ArtCraft name and logos are excluded and must not be used.
- **Most crates won't drop in.** PdfCraft builds on its own object model (`pdfcraft-cos`) and `lopdf`, not MuPDF. Expect
  to reuse logic rather than whole crates. Self-contained pieces are the likeliest candidates, such as:
  - XFDF parsing and writing;
  - CMS / PAdES signing and validation;
  - redaction verification;
  - the JSON-Schema tool table;
  - the parity checklist format.
- **Check each crate before copying.** Look at its dependencies first.

---

## 26. Appendix D: expert review additions

What a review by the strongest PDF, browser-engine and systems developers would add to this plan. Most items are
decisions that cost little now and a great deal later, so each one names where it goes in the plan and when.

### D.1 Architecture decisions to make before Phase 1

**1. Run the document engine out of process, sandboxed, from the start.** Tag: **K.22**.

- **Why:**
  - §14.2 plans to start in-process and move parsing out "later if fuzzing shows risk". That move is nearly impossible
    later, because §5.3 shares MuPDF display lists between threads, and display lists cannot cross a process boundary.
  - Chrome sandboxes PDFium for the same reason. A hostile or malformed PDF should crash one worker process, not the
    app with unsaved ink in twenty other tabs.
- **Design:**
  - The `Document` trait in `excellent-core` becomes message-based: an async request in, a response out.
  - Render processes write tiles into shared memory, and text and structure come back serialized.
  - In-process mode stays available behind the same trait for debugging and tests.
  - Sandboxing: AppContainer on Windows, App Sandbox on macOS, seccomp on Linux.
- **Plan changes:**
  - §5.1, §5.3 and §14.2 describe the process boundary.
  - Phase 0 gains a spike: render tiles in a child process and measure the cost against the 150 ms first-page budget
    (§13.1).
  - If that budget can't be met, record why in an ADR before falling back to in-process.

**2. Make one operation log the single source of truth for edits.** Tag: **K.14**.

- **Why:** the plan has four separate histories that will not reconcile cleanly once sync (Phase 9) sits on top of
  incremental PDF saves:
  - MuPDF's undo journal;
  - the recovery journal (§9.3);
  - CRDT sync (§11.12);
  - stroke samples and audio timestamps (§8.3, §11.6).
- **Design:**
  - Every edit is an operation with a stable id, an author, a timestamp, a target annotation id and its data.
  - The PDF is a materialized view of the log, written on save.
  - Undo, crash recovery, annotation history (§11.3), stroke replay, audio sync and CRDT sync all read the same log.
- **Plan changes:**
  - §9 defines the log format and a schema version.
  - Phase 1 fixes the design; Phase 3 implements it.
  - §11.12 builds sync on the log instead of on PDF diffs.

**3. Make the text layer its own subsystem.** Tag: **K.13**.

- **Why:** search, selection, read aloud, the dictionary, reflow, accessibility, AI citations and handwriting search
  all depend on extracted text. When the text is wrong, each of those features breaks in its own way.
- **Design:**
  - A new `excellent-text` crate between MuPDF's structured text and every feature.
  - It owns reading order and columns, hyphen joins across lines, ligature expansion and Unicode normalization.
  - It also owns bidirectional (right-to-left) text and word segmentation for languages without spaces, through ICU4X.
- **Plan changes:**
  - §5.4 adds the crate.
  - §16 adds a text-quality corpus with metrics: word F1 against `pdftotext`, a reading-order score, and CJK, Arabic
    and Hebrew files.
  - Phase 1 ships it.

**4. Support input methods (IME) on the canvas.** Tag: **U.8**.

- **Why:** free-text boxes, sticky notes and form fields are drawn inside the canvas's own child window (§7.3). NSH's
  text input does not reach them, so Chinese, Japanese and Korean input would not work in annotations.
- **Design:** the canvas platform layer gets a fourth duty, text input:
  - TSF on Windows;
  - `NSTextInputClient` on macOS;
  - IBus / Fcitx through GTK on Linux;
  - `UITextInput` on iPad and `InputConnection` on Android.
- **Plan changes:**
  - §7.2's list of platform-layer duties grows from three to four.
  - Phase 3's checklist adds a test with real IMEs.

**5. Anchor annotations so they survive document changes.** Tag: **K.28**.

- **Design:** every annotation gets two anchors:
  - a UUID in the PDF `/NM` field;
  - a text-quote anchor in the W3C Web Annotation style (prefix, exact text, suffix and an approximate position).
- **Why:**
  - Annotations survive new versions of a document, re-OCR and sync.
  - Personal bookmarks (§11.14) and research-canvas links (§11.10) reuse the same anchors.
  - Enhanced's one-off annotation transfer becomes a general feature.
- **Plan changes:**
  - §9.1 adds the anchor model.
  - §11.3 adds W3C Web Annotation JSON export next to XFDF.
  - Phase 3 implements it.

### D.2 Strategic risks

**6. AGPL conflicts with the Apple App Store.**

- **The conflict:** the App Store's terms are widely held to be incompatible with GPL-family licences (VLC was removed
  for this reason). An AGPL ExcellentPDF on iPad therefore needs one of these:
  - a commercial MuPDF licence;
  - distribution outside the App Store, which is possible only through EU alternative marketplaces.
- **Plan changes:**
  - Add this to §20 as a risk.
  - Add it to Q1 in §21, so the licence decision covers iPad distribution.
  - Decide before Phase 8 work starts. Google Play and direct Android distribution are not affected.

**7. Move an iPad canvas spike into Phase 0.**

- **Why:** pen-first users mostly work on iPads (GoodNotes, Notability). "Shared canvas core, thin platform layers"
  (§7.2) stays an assumption until it runs on UIKit with Apple Pencil coalesced and predicted touches.
- **Plan changes:**
  - Phase 0 gets a 1–2 week spike: pages plus pressure ink on an iPad, with latency measured.
  - Shipping stays in Phase 8.

**8. Narrow the first target and add cut rules.**

- **Why:** 56+ weeks for one developer with AI agents is this plan's largest risk, and §20 names scope without a rule
  for cutting it.
- **Plan changes:**
  - §1 states a first wedge, for example "the best study and annotation tool for students on Windows and iPad".
  - Each phase in §18 gets one measurable goal, and a rule for what is cut, in priority order, when the phase runs
    more than 25% over its estimate.

### D.3 Engineering process

**9. Release engineering from Phase 1.** Tag: **K.25**.

- **Updates:** signed delta updates with rollback, using TUF through the `tough` crate. Stable, beta and nightly
  channels, with staged rollout.
- **Crash reports:** opt-in minidumps, symbolicated against a private symbol server, with no document content.
- **Supply chain:** reproducible builds, an SBOM per release, `cargo vet` alongside `cargo deny`, and signed release
  artifacts.
- **Plan changes:** §14.2 and §17 (a `release` workflow), and the Phase 1 checklist.

**10. Performance data from real use.** Tag: **K.24**.

- **Tracing:** replace the `log` facade with `tracing` spans that export to Perfetto or Tracy.
- **Traces from users:** a "Record performance trace" command lets a user attach a trace to a bug report. It contains
  timings and no document text.
- **Energy budgets:** add desktop budgets to §13.1: zero idle wakeups, no running timers while idle, and no GPU work
  when nothing changes. Battery is currently measured only on tablets.
- **Plan changes:** §13.

**11. Model-based testing of edits and saves.** Tag: part of **K.14** (tests).

- **Tests:**
  - Generate random sequences of annotation, ink and page operations; save, reopen, and compare the result with an
    in-memory model.
  - Inject a crash at every write point.
  - Replay the same sequences through the interop readers (§16).
- **Why:** this finds data-loss bugs that hand-written round-trip tests miss. It also tests the operation log of D.1.2.
- **Plan changes:** §16 (`proptest`-based), from Phase 3.

### D.4 Features to add

| Tag | Feature | Priority | Where |
| --- | --- | --- | --- |
| **O.24** | Colour-accurate and print-production view: display ICC profiles, output intents, overprint preview, separations (all supported by MuPDF) | P2 | §11.11 |
| **R.30** | Open from a URL with progressive loading of linearized files over HTTP range requests (read an arXiv paper while it downloads) | P1 | §11.2 |
| **U.7** | Localization from day one: NSH's translations (`nsh-i18n`, rust-i18n), a mirrored UI for right-to-left languages, vertical CJK text in ebooks | P0 infrastructure, P1 languages | §10, §15, Phase 1 |

### D.5 Changes by phase

| Phase | Additions from this appendix |
| --- | --- |
| 0 | Out-of-process render spike (D.1.1); iPad canvas spike (D.2.7); `tracing` set up (D.3.10) |
| 1 | Operation-log design (D.1.2); `excellent-text` with ICU4X (D.1.3); signed updates, channels, crash reports (D.3.9); localization infrastructure (D.4) |
| 3 | Operation log implemented; IME on the canvas (D.1.4); annotation anchors and W3C export (D.1.5); model-based edit tests (D.3.11) |
| 5 | Open from URL with progressive loading (D.4) |
| 7 | Colour and print-production view (D.4) |
| Before 8 | Licence decision covers App Store distribution (D.2.6) |
| Every phase | One measurable goal and a cut rule (D.2.8) |

---

## 27. Implementation plans by feature tag

Every feature in this plan has a **tag**: a letter for its area and a number, for example **A.2** (annotation feature
2). Tags appear next to each feature in §11, §12.1 and Appendix D. Each tag has one implementation plan in this
section; find it by searching for its tag after the bar, for example `| A.2:` (heading
`#### Ph3: 15/49 | A.2: ...`).

### 27.0 How to use this section

**Tag letters**

| Letter | Area | Features listed in | Plans in |
| --- | --- | --- | --- |
| K | Core and foundations (kernel) | §5–§9, §13, §14, Appendix D.1 and D.3 | 27.1 |
| R | Reading and navigation | §11.1, §11.2 | 27.2 |
| B | Bookmarks and outline | §11.14 | 27.3 |
| V | Split view | §11.15 | 27.4 |
| W | Workspaces | §11.16 | 27.5 |
| A | Annotation | §11.3 | 27.6 |
| H | Handwriting, ink and drawing | §8, §11.4 | 27.7 |
| N | Notebooks and notes | §11.5 | 27.8 |
| L | Audio and lectures | §11.6 | 27.9 |
| S | Study and language | §11.1, §11.7 | 27.10 |
| G | Teaching and presenting | §11.8 | 27.11 |
| F | Find: search, library and citations | §11.9 | 27.12 |
| X | Excerpt (research) canvas | §11.10 | 27.13 |
| O | PDF tools | §11.11, §14.3 | 27.14 |
| Y | Sync and sharing | §11.12 | 27.15 |
| E | Extensibility and automation | §11.13 | 27.16 |
| I | AI | §12 | 27.17 |
| U | Universal access: accessibility, read aloud, languages | §15, Appendix D.4 | 27.18 |

Letters not used, to avoid confusion: **C** and **D** (Appendix C and D sections), **P** (priorities P0–P3), **Q**
(questions Q1–Q7), **T** (tasks T1–T10). Decisions D1–D10 in §3 have no dot, so they don't clash with tags.

**Tag rules**

- A tag never changes. A new feature takes the next free number in its area. A dropped feature keeps its tag and is
  marked *(dropped)*.
- **Labels.** Every entry's heading starts with its phase label, `Ph<phase>: <n>/<count>`: its place among its
  phase's entries, counted in this section's order, then its tags and titles: `#### Ph1: 7/60 | R.10: Recent and
  pinned files`. Feature docs, `docs/features.toml`, changelog headings and progress reports use the same label, so
  the reader sees how far each phase is. Tags stay the permanent names; when an entry is added, its phase's labels are
  counted again and updated everywhere.
- Several tags share one heading when they share code, for example
  `#### Ph4: 39/44 | G.2: Laser with trails; G.3: Spotlight; G.4: Magnifier`.
- Use the tag in specs, branch names, commit and pull-request titles, test names and the feature checklist (K.27),
  for example `feat(A.2): replace-text markup`.

**Entry format**

- **First line:** priority; phase; **Needs** (tags that must ship first); **Code** (where it lives).
- **Steps:** in order. Each step is small enough for one agent session and ends in something testable.
- **Done when:** the acceptance checks. A feature is not done until they pass.
- **Links:** specifications, libraries and reference code, where useful.

**Workflow for an agent picking up a tag**

1. Read the entry, the plan sections it cites, and the entries of its **Needs**. Check in `docs/features.toml` (K.27)
   that every need is `shipped`. If one isn't, build the need first or report it; don't stub it silently.
2. Write `docs/features/<tag>-<slug>.md` from `_template.md` (problem, user story, UI, mode, speed and size impact,
   acceptance tests). Copy **Done when** into its acceptance tests.
3. Build the model first: pure Rust, unit-tested (`cargo test -p <crate>`), with no UI or OS types (§17).
4. Then the view (NSH components) and the command: register it in the command table (K.16), so it has a menu or
   toolbar entry, a shortcut where useful, and a palette entry.
5. Run `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `nsh test` and the budget checks (K.24).
   Fix regressions before going on.
6. Set the tag's row in `docs/features.toml` to `shipped`, naming the test that proves it. CI rejects the claim if
   that test doesn't exist.
7. One tag per pull request where possible, with the tag in its title.

**Rules for every feature (not repeated in each entry)**

- Every change to a document is one undoable operation in the operation log (K.14) and is saved by K.15.
- Every action is reachable three ways: menu or toolbar, keyboard, command palette (§2.1, rule 3).
- All UI text goes through the localization layer (U.7). Every control has an accessible name and keyboard access (U.1).
- Nothing leaves the device without the user's consent. Network features check the privacy settings and work, or
  fail clearly, offline (§14.4).
- Long jobs run on background workers with progress and cancel (§14.1), and never block the UI thread (§5.3).
- New settings have defaults, live in `settings.toml` (K.16) and appear in the settings UI.
- No budget in §13 may get worse by more than 10 %.
- Downloads and packs are signed and verified (K.21). User files are written atomically (K.15).

**Shared links**

- MuPDF documentation: <https://mupdf.readthedocs.io/en/latest/>. The headers in `third_party/mupdf/include/mupdf/`
  are the source of truth for 1.28.5. Every MuPDF function named in this section exists there.
- MuPDF usage examples: `third_party/mupdf/docs/examples/` (`example.c`, `multi-threaded.c` and JavaScript samples)
  and the `mutool` sources in `third_party/mupdf/source/tools/`.
- PDF 2.0 (ISO 32000-2), free through the PDF Association: <https://pdfa.org/sponsored-standards/>
- PDF 1.7 (ISO 32000-1) from Adobe: <https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf>
- NSH documentation: <https://nsh.dev>, and the source `C:\dev\nsh-src` at v0.5.0 (`website/docs/`, `docs/dev/`).
- SumatraPDF Enhanced, for anything ported: source in `C:\dev\sumatrapdf\src\`; every feature, setting and command
  with implementation notes in `dev notes/sumatra_pdf_enhanced_plan.md`.

### 27.1 Foundations (K)

#### Ph0: 1/13 | K.1: Repository and workspace skeleton
**P0; Phase 0; Needs:** none; **Code:** repository root; task T1
1. Create the folders of §17 and the crates of §5.4 as empty libraries, each with a `//!` doc comment.
2. Add `rust-toolchain.toml` (pinned stable), root `[workspace.dependencies]`, the workspace lints of §17, and
   `deny.toml`.
3. Add `AGENTS.md` (the §17 rules plus the workflow in §27.0), `docs/features/_template.md`,
   `docs/adr/0000-template.md`, and one ADR per decision D1–D10.

**Done when:** `cargo build --workspace` and `cargo deny check` pass on a clean clone.
**Links:** cargo-deny <https://github.com/EmbarkStudios/cargo-deny>

#### Ph0: 2/13 | K.2: MuPDF build (`excellent-mupdf-sys`)
**P0; Phase 0; Needs:** K.1; **Code:** `crates/excellent-mupdf-sys`, `third_party/`; tasks T2, T3
1. Export upstream SumatraPDF's tree into `third_party/`: in `C:\dev\sumatrapdf`, `git fetch upstream master`, then
   `git archive upstream/master ext/mupdf ext/patches src/shared/mupdf` (70 patches). Record the commit and the
   MuPDF tag in `third_party/README.md`.
   Add Enhanced's `0048`–`0053` as `0088`–`0093` (§6.1); rebase `0091` and `0093` on upstream's `pdf-appearance.c`.
   Verify the tree equals MuPDF 1.28.5 plus all 76 patches with `ext/patches/README.md`'s procedure.
2. Copy upstream's `ext/a-*` amalgamations into `third_party/amalgamations/`, plus libjpeg-turbo, cmark-gfm and jxldec.
3. Write `build.rs` with the `cc` crate. Take the source list and defines from Sumatra's `mupdf_files()` in
   `premake5.files.lua`. Built-in fonts: Sumatra ships them in a compressed embedded archive instead of compiling
   them in (see the `IDR_EMBEDDED_PAK` notes in `premake5.lua` and `src/mupdf/noto_sumatra.c`); do the same to stay
   under the size budget.
4. Write `shim.c` / `shim.h`: one C function per MuPDF call used, each inside `fz_try` / `fz_catch`, returning an
   error code and copying the message into a caller buffer (§6.3).
5. Run `bindgen` on `shim.h` and the public MuPDF types only; commit `src/bindings.rs`, so builds need no libclang.

**Done when:** a clean clone builds on Windows (MSVC), and a smoke test opens a corpus PDF and counts its pages.
**Links:** cc <https://docs.rs/cc>, bindgen <https://rust-lang.github.io/rust-bindgen/>, mupdf-rs's build
<https://github.com/messense/mupdf-rs/tree/main/mupdf-sys>

#### Ph0: 3/13 | K.3: Safe Rust API (`excellent-mupdf`)
**P0; Phase 0; Needs:** K.2; **Code:** `crates/excellent-mupdf`; task T4
1. Types with `Drop`: `Context`, `Document`, `Page`, `DisplayList`, `Pixmap`, `StextPage`, `Annot`, `Writer`,
   `Outline`. Every call returns `Result<T, MuError>` with the message from the shim.
2. Typed geometry: `PagePoint`, `DevicePoint`, `PageRect`, `Quad`; one `Transform` type applying rotation, crop box
   and zoom (`excellent-core::transform`).
3. Thread rules in the types: `Context` is `!Send`; `DisplayList` is `Send + Sync`; a `Document` sits behind a mutex.
4. A CLI test: render page 1 of every corpus file to PNG and extract its text.

**Done when:** the CLI test passes leak-free under ASan, and the first fuzz target (open + render) runs in CI.
**Links:** cargo-fuzz <https://rust-fuzz.github.io/book/cargo-fuzz.html>
**Built (T4, 2026-11-13):**
- The context is not a public type: one locked base context per process and a lazily cloned one per thread
  (`thread_local!`), which every call uses. This keeps "`Context` is `!Send`" true by construction and saves the
  render workers (K.4) from passing contexts around.
- `Document::outline()` returns an owned `Vec<OutlineItem>` tree, copied out of MuPDF at once; `Document::from_bytes`
  opens a document held in memory (used by the fuzz target).
- The built-in fonts (URW base 14, Charis SIL; 0.8 MB) are compiled in uncompressed, finishing K.2 step 3. Compressing
  them is a size task for T10. *T10:* not needed for the budget. The installer compresses the exe (LZX), fonts
  included, and is 11.79 MB.
- The CLI test is `crates/excellent-mupdf/tests/corpus.rs`. ASan and fuzzing run in `.github/workflows/fuzz.yml`
  (Linux, weekly and by hand, DD2), not yet on a developer machine: they need the nightly toolchain.

#### Ph0: 4/13 | K.4: Render workers and threading
**P0; Phase 0; Needs:** K.3; **Code:** `excellent-core::render`; task T5
1. Base context with real `fz_locks_context` locks; a worker pool of `cores − 1` threads, each with
   `fz_clone_context` (§5.3).
2. Jobs: `BuildDisplayList(page)`, `RenderTile(list, rect, zoom)`, `ExtractText(page)`. Each carries an `fz_cookie`
   and a priority (§7.4, item 5). Cancelling sets `cookie.abort`.
3. Stress test: four workers render display lists of one document 10,000 times.

**Done when:** the stress test passes under ASan, and cancelling a job stops it within 20 ms.
**Built (T5, 2026-11-13):** `excellent-core::render::RenderPool` with `build_display_list`, `render_tile` and
`extract_text`; each returns a `Job` that can be cancelled (dropping it cancels too) or waited on. `excellent_mupdf::Cookie`
wraps `fz_cookie`, with `abort` set atomically. Priorities are `render::Priority`, in §7.4's order. Documents are shared
as `Arc<Mutex<Document>>` until K.5's document model owns them. Tiles render on white; dark mode (K.9) adds its own
background. Tests: `crates/excellent-core/tests/render_stress.rs`. Not built yet: pausing background work while the
user scrolls or writes (§7.4 item 5), which needs the canvas (T7), and a non-blocking way to collect results, which T7
adds with the UI thread's wake-up.

#### Ph1: 1/60 | K.5: Document and View model, document registry
**P0; Phase 1; Needs:** K.4; **Code:** `excellent-core::{document, view, registry}`
1. `Document`: one open file, holding the MuPDF document, display-list cache, annotation model, operation log, dirty
   flag and revision counter per page (§11.15, "How it works" 1).
2. `View`: viewport, zoom, rotation, layout mode, selection, find state, history, dark-mode override. Serializable,
   for sessions and workspaces.
3. `Registry`: file identity → `Arc<Document>`, reference-counted. File identity is path, size, modified time and a
   hash of the first and last 64 KB (§11.16, "How it stays efficient" 1).
4. Hibernate and wake: drop display lists and tiles, then close the MuPDF document; never while it has unsaved
   changes.

**Done when:** two Views of one Document share one parse and its caches; a file opened twice is parsed once; tests
cover hibernating with and without unsaved changes.

**Built (2026-11-14), items 1, 3 and 4:** `excellent-core::{document, registry, view}`. `Document` holds the shared
MuPDF document, each page's display list and revision, and the unsaved flag; a list built against an older revision is
refused. `document::FileId` is the canonical path, size, modified time and an FNV-1a hash of the first and last 64 KB;
`Registry` maps it to a `Weak<Document>`, so a file opened twice is parsed once and freed with its last view. The page
canvases share one process-wide registry. `hibernate` drops the lists and closes the MuPDF document, and is refused
while there are unsaved changes; `mupdf()` wakes it. `View` (spec, rotation, page box, history) moved here from the
canvas, with `ViewCommand` and `Effect`. Tests: `a_file_opened_twice_is_parsed_once_and_shared`,
`hibernates_only_without_unsaved_changes`, `a_changed_page_drops_its_list_and_refuses_stale_ones`,
`one_file_has_one_identity_and_new_content_a_new_one`. Open: `View` is not serialized yet (R.9 chooses the format); the
live viewport, selection and find state stay in the canvas until they exist; the annotation model and operation log
come with K.13 and K.14; tiles are still per canvas (K.7), so two views share lists but not tiles; the registry lock is
held while a file is parsed (shortcut: fine for one window, split the lock when tabs (R.12) open files in parallel).

#### Ph1: 2/60 | K.6: Layout and coordinate transforms
**P0; Phase 1; Needs:** K.3; **Code:** `excellent-core::layout`, `excellent-core::transform`
1. Page sizes and rotations only on open; never load every page.
2. Layouts as pure functions from page sizes, viewport size and mode to page rectangles: single, continuous, facing,
   book (cover alone), horizontal, grid, right-to-left (R.1, R.3).
3. Transforms between page space and screen space for any zoom, rotation and crop box.
4. Property tests (`proptest`) that page → screen → page round-trips, and that pages never overlap.

**Done when:** property tests pass; laying out 10,000 pages takes under 5 ms.
**Links:** proptest <https://github.com/proptest-rs/proptest>

**Built (2026-11-14):** in `crates/excellent-canvas/src/layout/` (`spec`, `arrange`, `pages`, `properties`), not
`excellent-core::layout`: it has no MuPDF types, so every platform's canvas shares it. Item 1: `Document::page_box`
reads a PDF page's box from the page tree through the shim's `ex_page_box`, without loading the page; other formats
still load each page once on open. Item 2: `Layout::new(sizes, Spec { mode, flow, direction })` gives single, facing,
book (cover alone), horizontal and grid, each continuous or paged (paged: single, facing and book), left to right or
right to left. Pages sit in bands (rows, or the strip's columns), so `pages_in` and `page_at` are binary searches. The
viewport size is not an input: zoom is the viewport's, and the canvas picks the grid's columns from the window width.
Item 3: `excellent-core::transform` gained `to_page` (the inverse) and `shown_size`; `excellent-canvas-win::placement`
maps page points to layout points under the view rotation. Item 4: proptests `pages_never_overlap_and_stay_inside`,
`bands_hold_every_page_once_in_order`, `queries_find_every_page` and `page_to_device_and_back`. Measured
(`lays_out_ten_thousand_pages_quickly`, release, `--ignored`): 10,000 pages in 0.19 ms (grid) to 0.49 ms (single).
Open: non-PDF formats load each page for its size.

#### Ph0: 5/13 | K.7: Tile cache and render scheduler
**P0; Phase 0–1; Needs:** K.4, K.6; **Code:** `excellent-core::tiles`
1. Tiles of 256 × 256 device pixels; zoom quantized to buckets; the cache key of §7.4, item 6.
2. Display-list LRU and tile LRU, each with a byte budget; evict on OS memory-pressure events.
3. Scheduler: visible tiles first, then the priorities of §7.4, item 5; cancel jobs for pages that leave the viewport.
4. Progressive rendering: a low-resolution page bitmap first, then sharp tiles; scale existing tiles during pinch and
   re-render when the gesture settles.
5. Edits bump the page revision; only that page's tiles are invalidated. The annotation being edited is drawn live,
   not baked into tiles (§7.4, item 8): render with `fz_new_display_list_from_page_contents` plus
   `fz_run_page_annots` for the others.

**Done when:** scrolling a 1,000-page vector-heavy PDF holds 60 fps on the reference machine; memory stays within
budget; a recorded scroll trace shows no UI-thread stall over 50 ms.

**Built (T7, 2026-11-13), part of this tag:** in `crates/excellent-canvas` (not `excellent-core::tiles`: it has no
MuPDF types, so every platform's canvas uses it) the continuous `layout`, the `viewport` (scroll, zoom about a point,
fit width, SumatraPDF's zoom steps) and `tiles` (256-pixel tiles, 1/4096 zoom buckets, a cache that cancels tiles no
longer needed and evicts the least recently drawn over a byte budget, and older-zoom tiles drawn while new ones
render). `excellent-core::render::Job` gained `try_take` and `when_done`, so the UI thread is woken instead of waiting.
Not built yet: the cache key's other fields (item 1), the display-list byte budget and memory-pressure eviction (item
2; lists are kept for pages within 8 of the screen), lower priorities for the next screen (item 3), progressive
rendering (item 4) and revisions (item 5). Unmeasured: the 60 fps, memory and stall targets wait for K.24's harness.

#### Ph0: 6/13 | K.8: Page canvas on Windows
**P0; Phase 0–1; Needs:** K.7; **Code:** `crates/excellent-canvas`, `crates/excellent-canvas-win`; task T7
1. Register `excellent.page-canvas` as an NSH native component that owns a child `HWND` (§7.1, §7.3).
2. Present through a DXGI flip-model swap chain with a waitable object; draw tiles and overlays with Direct2D.
3. Input: `WM_POINTER*` with `GetPointerPenInfo` and `GetPointerPenInfoHistory` (for H.1); wheel, keyboard focus.
   Port Enhanced's `src/PointerInput.h` (`NormalizePointer`, `PointerSample`: pressure from Windows' 0–1024
   scaled to 0–1, tilt, rotation, barrel, eraser, inverted, coalesced history samples) to a Rust `PointerSample` in `excellent-canvas-win`; no NSH change is needed (§7.3).
4. Overlays drawn inside the canvas: selection, search hits, annotation handles, ink preview, laser, floating
   selection toolbar (§7.3).
5. Fallback: if the swap chain fails to start, blit through GDI (Appendix C.4).
6. JSON props (document id, view state) and events (page changed, selection, link clicked) to the NSH app.

**Done when:** a document scrolls and wheel-zooms smoothly inside the NSH window; the GDI fallback works when
forced by a setting.
**Links:** NSH native components `C:\dev\nsh-src\website\docs\native-components.md`; flip model
<https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-flip-model>; pointer input
<https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfo>

**Built (T7, 2026-11-13):** items 1 and 2, and the wheel part of item 3. `excellent-canvas-win` registers
`excellent.page-canvas`; its window is a hidden popup that NSH adopts as a child of the app window and places over the
view. It draws with Direct2D (96 dpi, one unit per pixel) on a D3D11 device (WARP when there is no GPU), into a
flip-model swap chain with a frame-latency waitable object and a latency of one frame. Props: `path` (the document) and
`label` (the accessible name, also the window's title for screen readers). Commands: `zoomIn`, `zoomOut`,
`actualSize`, `fitWidth`, `screenDown`, `screenUp`, which the app's command table sends through an `element_ref`
(Ctrl+Plus, Ctrl+Minus, Ctrl+1, Ctrl+2, Page Down, Page Up). Wheel scrolls; Shift+wheel and the tilt wheel scroll
across; Ctrl+wheel zooms about the pointer. The document opens on a background thread, which reads every page's size;
display lists and tiles render on the shared `RenderPool`, which wakes the window with a posted message. Findings: the
canvas window covers anything NSH draws over its view, so the app hides it while the command palette is open (a
general fix is T9's upstream proposal); `DefWindowProc` must run without the canvas borrowed, because its handling of
`WM_WINDOWPOSCHANGED` sends `WM_SIZE` back at once. Shortcuts, each marked in the code: one process-wide render pool
until K.5, the startup document from the command line until R.11, tiles turned from RGB to BGRA on the UI thread, and
an open failure that is only logged. Not built yet: pointer and pen input (T8, H.1), keyboard focus (the app keeps it
and sends commands), overlays (item 4), the GDI fallback (item 5) and events (item 6).

**Built (T8, 2026-11-13), the pen spike:** `pointer.rs` ports Enhanced's `PointerInput.h` (the same sample, with
`Option` for values the pen doesn't report, and its unit tests); `WM_POINTERDOWN`, `WM_POINTERUPDATE` (every coalesced
sample through `GetPointerPenInfoHistory`) and `WM_POINTERUP` draw a stroke whose width follows pressure
(`excellent-ink::stroke`, 0.4 to 2.4 points), and touch and mouse go on to the default handling. Each pen message hands
its samples to the compositor's ink trail (§7.3) and draws our frame at once if the swap chain takes one without
waiting, else on the next `WM_PAINT`; the window turns off Windows' pen tap and press-and-hold visuals. Latency: each
stroke logs, at debug level, the time from the newest sample's input timestamp to `Present` returning (not to the
pixels: that adds up to one refresh and isn't visible to the app). The swap chain moved from `CreateSwapChainForHwnd`
to `CreateSwapChainForComposition` for the trail. Shortcuts, marked in the code: one pen, one colour, linear pressure
(H.1, H.2); the eraser end does nothing (H.5); strokes stay in memory until the document closes, not saved (H.1's
commit). Not measured: the "one display frame" target, which needs a pen; the reference machine for it is the
maintainer's Surface Pro, tested by hand, not with injected input.

#### Ph2: 1/3 | K.9: Page canvas on macOS
**P0; Phase 2; Needs:** K.8; **Code:** `crates/excellent-canvas-mac`
1. `NSView` backed by a `CAMetalLayer`; upload tiles as Metal textures; draw overlays with Metal or Core Graphics.
2. Tablet and trackpad events: pressure and tilt from `NSEvent`, magnify and scroll gestures.
3. Same JSON props and events as K.8; the shared logic stays in `excellent-canvas`.

**Done when:** Phase 1's reading checklist passes on macOS.
**Links:** objc2 crates <https://github.com/madsmtm/objc2>; CAMetalLayer
<https://developer.apple.com/documentation/quartzcore/cametallayer>

#### Ph2: 2/3 | K.10: Page canvas on Linux
**P0; Phase 2; Needs:** K.8; **Code:** `crates/excellent-canvas-gtk`
1. A GTK 4 widget that draws tiles as `GdkTexture`s through `GtkSnapshot`.
2. Pen input through `GtkGestureStylus` (pressure, x and y tilt); touch through GTK gestures.
3. Test on Wayland and X11.

**Done when:** Phase 1's reading checklist passes on GNOME (Wayland) and one X11 desktop.
**Links:** gtk4-rs <https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/>

#### Ph0: 7/13 | K.11: Page canvas on iPad
**P1; Phase 0 spike, Phase 8 ship; Needs:** K.8; **Code:** `crates/excellent-canvas-ios`
1. Phase 0 spike (Appendix D.2.7): a `UIView` with a `CAMetalLayer` showing tiles, plus pressure ink from
   Apple Pencil using coalesced and predicted touches. Measure latency.
2. Phase 8: pinch, pan, double-tap zoom; Pencil hover; palm rejection by touch type; Scribble off on the canvas.

**Done when:** the spike's latency numbers are written down (Phase 0); the Phase 1 reading checklist passes on iPad
(Phase 8).
**Links:** UITouch <https://developer.apple.com/documentation/uikit/uitouch>

#### Ph8: 1/3 | K.12: Page canvas on Android
**P1; Phase 8; Needs:** K.8; **Code:** `android/`, `crates/excellent-canvas-android`
1. A Kotlin `View` with a `SurfaceView`, calling Rust over JNI for tiles and model updates.
2. Input from `MotionEvent`: pressure, `AXIS_TILT`, `AXIS_ORIENTATION`, historical samples,
   `requestUnbufferedDispatch`.
3. Low-latency ink with androidx graphics front-buffered rendering, and motion prediction.

**Done when:** the Phase 1 reading checklist and the ink latency budget pass on a reference stylus tablet.
**Links:** low-latency stylus <https://developer.android.com/develop/ui/views/touch-and-input/stylus-input>;
androidx graphics <https://developer.android.com/jetpack/androidx/releases/graphics>; jni crate
<https://docs.rs/jni>

#### Ph1: 3/60 | K.13: Text layer (`excellent-text`) and selection
**P0; Phase 1; Needs:** K.3; **Code:** `crates/excellent-text` (Appendix D.1.3)
1. Extract with `fz_new_stext_page_from_page` and flags `FZ_STEXT_DEHYPHENATE`, `FZ_STEXT_SEGMENT`,
   `FZ_STEXT_COLLECT_STRUCTURE`, `FZ_STEXT_ACCURATE_BBOXES`; keep glyph quads.
2. Reading order: use the PDF's structure tags when present, else column detection on block positions.
3. Normalize: expand ligatures, join hyphenated words, Unicode NFC; word and sentence breaks with ICU4X
   (`icu_segmenter`), including languages written without spaces; bidirectional text for Arabic and Hebrew.
4. Selection model: character, word, line, paragraph and rectangle; across columns in reading order. Quads come from
   `fz_highlight_selection` and `fz_snap_selection`; text from `fz_copy_selection`, then normalized.
5. Text-quality corpus (§16): word F1 against `pdftotext`, a reading-order score, CJK, Arabic and Hebrew files.

**Done when:** selection and copy work on the corpus with scores at or above `pdftotext`'s; double-click selects a
word in Chinese and Thai text.
**Links:** ICU4X <https://github.com/unicode-org/icu4x>

**Built (2026-11-14), steps 1 and 4 in part:** the shim walks the structured text (struct blocks included, depth first)
into plain arrays (`ex_stext_copy`), which `excellent-mupdf::TextLayout` holds; `RenderPool::read_text` reads it on
the workers. Flags: `FZ_STEXT_DEHYPHENATE`, `FZ_STEXT_SEGMENT`, `FZ_STEXT_COLLECT_STRUCTURE`, **not**
`FZ_STEXT_ACCURATE_BBOXES`: its glyph-tight boxes make a highlight ragged, so characters keep the font's ascent and
descent. **Changed from step 4:** selection runs in Rust over the copied quads (`excellent-text::PageText`: caret and
character hit testing, word, line and paragraph snapping, one highlight quad per line, copy with line and block
breaks, re-joining words MuPDF de-hyphenated; `Selection` across pages), not through `fz_highlight_selection` / `fz_snap_selection` /
`fz_copy_selection`: a MuPDF text page can't leave the worker that made it, and selection must follow a drag on the
UI thread. The Windows canvas reads the text of the pages on screen and of every selected page, shows the I-beam over
text, and selects by drag, double click (words), triple (lines) and quadruple (paragraphs), drawn darkest-wins so text
stays black; **Copy** (`Ctrl+C`, `Ctrl+Insert`) and **Select all** (`Ctrl+A`, the page shown) are commands, and the
app puts the text on the clipboard. Open: steps 2 and 3 (reading order beyond MuPDF's, normalization, ICU4X words:
until then a CJK character is a word and a Thai run is one), rectangle selection, drag past the window's edge
scrolling, select all of the document, step 5's corpus scores. Tests: `excellent-text`'s
`the_caret_falls_between_the_characters_either_side_of_the_pointer`,
`words_lines_and_paragraphs_snap_around_a_character`, `a_highlight_is_one_quad_per_line`,
`copied_text_breaks_lines_and_separates_blocks`, `a_word_hyphenated_across_lines_is_copied_whole`,
`a_selection_across_pages_takes_all_of_the_pages_between`, `copying_a_page_gives_the_same_words_as_mupdf`;
`excellent-mupdf`'s `a_text_layout_covers_its_characters_in_order`; `excellent-canvas-win`'s
`quick_presses_in_one_place_count_up`.

#### Ph1: 4/60 | K.14: Annotation model and operation log
**P0; Phase 1 design, Phase 3 build; Needs:** K.5; **Code:** `excellent-core::{annot, oplog}` (Appendix D.1.2)
1. Annotation model: stable id (`/NM` UUID), type, page, rectangle and geometry in normalized page space, style,
   author, dates, contents, tags, text anchor (K.28). Unknown types are kept untouched.
2. Operation log: every edit is `{op_id, author, time, target_id, kind, data}`, appended to an fsync'd file.
3. Apply each operation to the PDF inside `pdf_begin_operation` / `pdf_end_operation`, so MuPDF's journal matches
   the log; undo and redo call `pdf_undo` / `pdf_redo` and append inverse operations to the log.
4. The log feeds: crash recovery (K.15), history (A.27), stroke replay (H.22), audio sync (L.1) and sync (Y.1).
5. Model-based tests (`proptest`): random operation sequences, save, reopen, compare with the in-memory model
   (Appendix D.3.11).

**Done when:** model-based tests pass for 10,000 random sequences; undo after reopening works where MuPDF's
`pdf_save_journal` / `pdf_load_journal` allow it.

**Built (2026-11-14), the operation log, for outline edits:** steps 2 to 4 (`excellent-core::{edit, oplog}`,
`excellent-mupdf::edit`). An `Operation` holds any number of changes and is one undo step; it runs inside
`pdf_begin_operation` / `pdf_end_operation`, and a failed one is taken back. The log is one JSON line per operation,
undo, redo and save, synced before the edit counts, and starts with the file's fingerprint; it lives in the app
folder's `oplog/`. Deviation from step 2: no `author` in records yet (one local user; it comes with sync, Y.1).
Undo is Ctrl+Z and redo Ctrl+Shift+Z (Ctrl+Y is Enhanced's zoom). Doc: `docs/features/K.14-operation-log.md`.

**Built (2026-11-15), step 1, the annotation model:** `excellent-core::annot` (shape as an enum of markup, boxed,
ink, line and path kinds; style; `/NM` id; tags and the K.28 anchor in a private `/ExcellentPDF` key) and the
changes `annot.create`, `annot.update` and `annot.delete`. Step 5's model test passes for 10,000 random sequences.
Open: undo after reopening; author in records.

#### Ph1: 5/60 | K.15: Saving, autosave, recovery and external changes
**P0; Phase 1 (read-only) and Phase 3 (edits); Needs:** K.14; **Code:** `excellent-core::save`
1. Incremental save with `pdf_save_document` and `do_incremental = 1` when `pdf_can_be_saved_incrementally` allows
   it; otherwise a full save.
2. Atomic writes: write to a temp file in the same folder, reopen it to verify, then rename over the original.
3. Autosave: the operation log immediately; the PDF after 2 s idle, on tab switch and on close.
4. Recovery: on start, replay unsaved operations from the log over the last saved revision and offer the result.
5. Watch the file for outside changes; reload safely; on conflict offer "keep both".
6. When the user signs out or Windows shuts down, flush the operation log and save. NSH main (after v0.5.0, #1744)
   lets apps save their state when the session ends; until we move past v0.5.0, handle `WM_QUERYENDSESSION` /
   `WM_ENDSESSION` in the canvas crate.

**Done when:** killing the process at every write point (fault injection) never loses a committed operation; saved
files pass `qpdf --check`.
**Links:** qpdf <https://qpdf.readthedocs.io/>

**Built (2026-11-14), steps 1 to 4:** incremental save when MuPDF allows it, else the whole file, after which the
document is reopened from it and its undo history starts again. The file is written hidden beside the original, read
back, synced and renamed over it; documents are opened with shared delete so the rename works while they are open.
Autosave 2 s after the last edit, on tab switch, and synchronously at quit; Ctrl+S saves now. Recovery: saves are
incremental, so the file as the log began is the start of the saved file, and replaying the whole log over those
bytes (the base prefix) rebuilds the unsaved state; the reader is asked once per run to restore or discard it. Open:
step 5 (outside changes, "keep both"), step 6 (sign-out and shutdown), fault injection, `qpdf --check`. Doc:
`docs/features/K.15-saving-and-recovery.md`.

#### Ph1: 6/60 | K.16: Settings, command table and shortcuts
**P0; Phase 1; Needs:** K.5; **Code:** `excellent-app::{settings, commands}`
1. `settings.toml` with serde; write only values that differ from the default; ignore unknown keys; watch the file and
   reload. Portable mode keeps it next to the exe.
2. One command table: id, name, label, default keys, mode, "available when" condition. Menus, toolbars, the palette
   (NSH `Command`) and the keymap (NSH `keymap`) are all built from it.
3. User shortcut overrides in settings, with conflict detection in the shortcuts UI.
4. Import Enhanced's settings and shortcuts where they map (`SumatraPDFEnhanced-settings.txt`).

**Done when:** every shipped feature's commands appear in the palette with shortcuts; a test fails if a command has
no palette entry.
**Built (T6, 2026-11-13), the seed of this tag:** `app/nsh.json` and `crates/excellent-app` (binary `excellentpdf`).
The shell (`excellent-app::shell`) is a hidden-title-bar window with our own 40-point bar (sidebar toggle, app name,
a palette button where a browser has its address bar, inspector toggle) over a controlled NSH `dock_area`: document
in the center, sidebar (260 pt) on the left, inspector (300 pt) on the right. `excellent-app::commands` is the
command table's first form (id, name, label key, group, keys); the palette (NSH `command()` in a `dialog`) and the
keymap (`on_key`) are both built from it. Three commands so far: `palette.open` Ctrl+K / Ctrl+Shift+P (Enhanced's),
`view.sidebar` F12 (Enhanced's bookmarks toggle) and `view.inspector` Ctrl+Alt+B (VS Code's secondary side bar;
a chosen default). UI text is in `app/locales/en.json` (English only until U.7). Tests:
`crates/excellent-app/tests/shell.rs` (headless). App id `com.excellentpdf.app` is a placeholder. Not built yet:
settings, the "available when" condition, menus, user overrides.
**Built (2026-11-14), step 1 in part:** `excellent-app::settings`. `settings.toml` is kept as a TOML table: only values
that differ from the default are written, keys this version doesn't know are kept, and the file is written atomically
(temporary file, then rename). It lives in `<system data folder>/<app id>/` (`%APPDATA%\com.excellentpdf.app` on
Windows, where NSH keeps the app's own files), or next to the exe when a `settings.toml` is already there (portable
mode, SumatraPDF's rule). A file that doesn't parse is logged and never overwritten. Only `main` loads it, so tests
never touch the user's file. First keys: `theme` and `appearance` (K.17). Needs K.5, which isn't built; settings
don't depend on it. **Not built yet:** watching the file and reloading (step 1), steps 2 to 4.
**Built (2026-11-14), steps 2 and 3 in part:** each row of the table has keys for the whole window and keys for the
pages (`page_keys`), which work only while the pages have the keyboard focus; Enhanced's single-letter and plain keys
are page keys (`c`, `[`, `]`, `g`, `f`, `j`, `k`, Home, End, Backspace, Shift+Backspace, Enter, Shift+Enter,
Shift+Space). The "available when" condition is `CommandId::is_available`: page commands leave the palette while no
document is open. `excellent-app::keymap` resolves the defaults with the user's overrides from `settings.toml`
(`[shortcuts."<command>"]` with `keys` and `page_keys` lists; `[]` unbinds): the user's key wins a conflict, between
defaults the earlier row does; unparsable keys, keys the platform keeps (F11) and unknown command names are logged and
not bound. **Not built yet:** menus, toolbars and the "mode" column (step 2), Enhanced import (step 4).
**Built (2026-11-14), step 1 and the settings UI:** `settings::Live` follows the file: once a second it compares the
file's modified time (a shortcut: polling, not an OS file watch; NSH v0.5.0 has none) and reads an edit in, so the
theme, the reading settings and the keys change at once; the app's own saves don't trigger a reload, and a file broken
by an edit keeps the old settings until it is fixed. The keymap is resolved again only when `[shortcuts]` changes.
**Settings** (`app.settings`, Ctrl+,, in the palette) is a window of four sections, every change saved as it is
made: Appearance (theme and light or dark, which hand over to their pickers), Reading (ruler colour, height, strength
and invert, focus scope, auto-scroll speed), Links (`link_protocols`) and Shortcuts (every command's keys, page keys
quieter, the problems the keymap found, and a button that opens `settings.toml`). Not built: changing a key in the
window itself (keys are edited in the file), a toolbar or menu entry for Settings (with menus).
**Measured (T6, release build, Windows 11, 7 runs):** cold start to a visible window, median **863 ms**, against the
300 ms budget (§13.1). `main` starts 84 ms after process creation; NSH's own startup (`run_app` until the first frame)
takes most of the rest, about 1 s from `main` to NSH's first log line. Profiling it is T10's job. The release exe is
30 MB, with no release profile yet (LTO, strip); T10 too. *T10:* with the release profile, cold start is 632 ms and
the exe 36.6 MB (more of MuPDF is built since DD1); see K.24's measurements.

#### Ph1: 7/60 | K.17: Design tokens and themes
**P0; Phase 1; Needs:** K.1; **Code:** `app/themes/`, `excellent-app::theme`
1. Tokens of §10.3 on top of NSH's theme; mode accents; annotation colors.
2. Theme families as small TOML files; contrast check in CI (WCAG AA) for every theme.
3. Density setting (compact, comfortable) and UI scaling.
4. Live preview (maintainer, 2026-11-13): the theme and appearance pickers apply the highlighted choice to the whole
   window as the user moves through the list (arrow keys, pointer, typing to filter); Enter or a click keeps it,
   Escape or a press outside goes back to the theme in effect.

**Done when:** every theme passes the contrast check; switching themes needs no restart; moving through the theme
picker previews each theme and Escape restores the one in effect.
**Links:** WCAG contrast <https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>

**Built (2026-11-14), steps 1, 2 and 4:** `crates/excellent-app/src/theme/` and `app/themes/*.toml`.
- One palette (`theme::palette`): an OKLCH lightness table per variant (light, dark, high-contrast light and dark)
  coloured by a family's `hue`, `chroma` and `tint`. Surfaces take a faint tint, accents the full chroma; danger,
  success, warning, the five mode accents (§10.2) and the page area keep their own hues in every family. Annotation
  colours are fixed tokens. Tokens are shadcn's plus `page-area`, `success`, `warning`, `mode-*` and `annot-*`/`ink-*`.
- Seven families (Neutral, Aurora, Coral, Lagoon, Citrus, Grape, Forest), each with a high-contrast variant. A family
  file can override any token per variant. `every_theme_passes_contrast` checks every family, variant and appearance:
  AA (4.5:1 text, 3:1 controls) for the standard variants, 7:1 and 4.5:1 for high contrast. It runs in `cargo test`, so
  CI runs it.
- Commands `view.theme` and `view.appearance` (no default shortcut) open the pickers. Switching calls NSH's
  `set_tokens` and `set_color_scheme`: no restart. The page canvas draws the theme's `page-area` colour.
- The choice is kept in `settings.toml` (K.16) as `theme = "<family>[:high]"` and `appearance = "light" | "dark"`,
  and the root starts with it, so the first frame already has the user's theme.
- **Open:** step 3 (density and UI scaling: NSH 0.5.0 has only the system font scale); custom theme files from the
  user's config folder (the format supports them);
  which bundled UI font §10.3 means (the system UI font for now). The page area changes only after the picker closes,
  because the page canvas is a window of its own and hides under dialogs.

#### Ph1: 8/60 | K.18: Modes and toolbars
**P0; Phase 1; Needs:** K.16, K.17; **Code:** `excellent-app::modes`
1. Modes Read, Annotate, Write, Study, Present (§10.2), each with a default toolbar built from command ids.
2. Customize: drag and drop toolbar items; export and import presets as JSON.
3. Pen contact switches to Write mode when the setting allows (§2.1, rule 3).

**Done when:** each mode shows only its tools; presets round-trip.

Built (2026-11-15): `excellent-features::modes` (modes, toolbars, JSON presets) and `excellent-app::modes` (the mode
bar under the tabs, the customize dialog, the pen switch). Write and Study join the switcher once they have tools;
tools are text until an icon set exists (DD8).

#### Ph1: 9/60 | K.19: Platform services
**P0; Phase 1 (Windows), Phase 2 (macOS, Linux), Phase 8 (tablets); Needs:** K.1; **Code:** `crates/excellent-platform`
1. Traits: files and dialogs, print (O.22), clipboard (text, image, rich text), text-to-speech (U.5), key store,
   notifications, shell integration (file associations, "Open with"), single instance, memory-pressure events.
2. One implementation per OS; a fake implementation for tests.

**Done when:** feature code compiles against the traits only; each implementation has an integration test.

In progress (2026-11-14): `excellent-platform` has single instance (`instance`: a named pipe on Windows, a Unix socket
elsewhere, tested end to end) and file associations (`associations`), for R.11. The traits and the fake come with the
second implementation (Phase 2), when there is something to abstract over.
**Links:** keyring crate <https://github.com/hwchen/keyring-rs>

#### Ph1: 10/60 | K.20: Library database (`excellent-store`)
**P0; Phase 1; Needs:** K.1; **Code:** `crates/excellent-store`
1. `library.db` with `rusqlite`: documents, files, tags, collections, workspaces (§11.16), bookmarks (B.6), cards
   (S.4), notes.
2. Numbered migrations with a test that upgrades a database from every earlier version.
3. WAL mode; one writer thread; reads from any thread.

**Done when:** migration tests pass; a crash during a write leaves the database consistent.
**Links:** rusqlite <https://github.com/rusqlite/rusqlite>

Built (2026-11-14, `docs/features/K.20-library-database.md`). Step 1 holds files, workspaces, tabs, closed tabs and
tab groups for now: the tables of documents, tags, collections, bookmarks, cards and notes come with the features
that use them (B.6, S.4 and the rest), each in a migration of its own, so their schemas are designed with them. The
database sits next to `settings.toml` (the app's data folder, or the program's for a portable copy).

#### Ph4: 1/44 | K.21: Pack manager
**P1; Phase 4 (dictionaries need it), finished in Phase 5; Needs:** K.20; **Code:** `crates/excellent-packs`
1. A signed index (Ed25519) listing packs: id, version, size, hash, URL, licence.
2. Download over HTTPS with resume; verify size, hash and signature before install; install atomically.
3. "Remove pack" frees the space; features ask the manager whether their pack is present and offer to download it.
4. Offline: a pack file can be installed from disk.

**Done when:** a tampered pack is rejected; interrupted downloads resume; removing a pack leaves no files.

*Built (2026-11-25, Phase 4 part):* `crates/excellent-packs`: the signed index, the self-signed pack file (`.expack`:
signature then zip), resumable HTTPS download checked by size, hash and signature, atomic install into the pack's own
folder, removal, install from disk. The trusted key waits for the maintainer (DD27); the Settings page comes in
Phase 5.
**Links:** ed25519-dalek <https://github.com/dalek-cryptography/curve25519-dalek>

#### Ph0: 8/13 | K.22: Out-of-process engine and sandbox
**P0; Phase 0 spike, Phase 1 build; Needs:** K.4; **Code:** `excellent-core::engine`, `excellent-engine-host` (Appendix D.1.1)
1. Spike: render tiles in a child process into shared memory; measure the cost against the 150 ms first-page budget.
   Write the result as an ADR.
2. Make the `Document` trait message-based (request → response), with in-process and out-of-process back ends.
3. Sandbox the engine process: AppContainer (Windows), App Sandbox (macOS), seccomp (Linux).
4. Crash handling: a dead engine process restarts and the tab reloads at the same place, with unsaved operations
   replayed from the log.

**Done when:** a crafted PDF that crashes MuPDF only reloads its own tab; budgets still pass.
**Links:** Chromium's sandbox design <https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md>

#### Ph0: 9/13 | K.23: Security limits and fuzzing
**P0; Phase 0 onward; Needs:** K.3; **Code:** `excellent-core::limits`, `tests/fuzz/`
1. Limits on image size, object counts, recursion and decompression; enforce them in the shims or in MuPDF's
   allocator callbacks.
2. `cargo fuzz` targets: open, render, text, save, annotation edit. Nightly in CI; each crash gets a regression test.
3. Links and attachments never open without confirmation; attachment names are checked for path traversal.

**Done when:** nightly fuzzing runs; no open fuzz crash older than a week.

#### Ph0: 10/13 | K.24: Performance instrumentation and budgets
**P0; Phase 0; Needs:** K.1; **Code:** `tests/perf/`, `tests/size/`, `tracing` spans everywhere (Appendix D.3.10)
1. Use `tracing` spans, exportable to Perfetto (Chrome trace JSON) or Tracy.
2. Benchmarks for each §13.1 budget on the corpus; `budgets.json` files; CI fails on a regression over 10 %.
3. A "Record performance trace" command that saves a trace with timings only, no document text.
4. Desktop energy checks: no timers or GPU work while idle.

**Done when:** CI reports every budget on each pull request.
**Links:** tracing <https://github.com/tokio-rs/tracing>; Perfetto <https://perfetto.dev>; Tracy
<https://github.com/wolfpld/tracy>

**Built (T10, 2026-11-13), part of item 2:** `tests/budgets.ps1` measures a release build against the two
`budgets.json` files, prints a table (and writes it to the GitHub job summary), and fails only when a measurement is
more than the tolerance (10 %) worse than its recorded `baseline`; being over a budget is reported, not failed, since
Phase 0 starts over some. The `budgets` job in `windows.yml` builds the release app and runs it on every pull request.
Three budgets so far:
- *Core installer:* the exe in a cabinet with LZX:21, as an MSI stores it (Windows' `makecab`), until K.25 builds the
  installer. Baseline 11.79 MB.
- *Cold start:* process creation to a visible top-level window, median of 7 launches with no document.
- *Open a typical PDF:* the canvas logs the time from being given the document to drawing its first page complete;
  the script reads that line. The document is `tests/corpus/zlib-manual.pdf` (2 pages) until K.26 adds a longer one.
Cold start and open have no baseline yet: the CI runner's numbers will be the baseline, not a developer machine's.
Not built yet: `tracing` spans (item 1), the other budgets, the trace command (item 3) and the energy checks (item 4).

**Measured (T10, release build with the profile, maintainer's PC, Windows 11 build 26300, 7 runs each):**

| Budget | Measured | Budget |
| --- | ---: | ---: |
| Core installer (LZX cabinet of the exe) | 11.79 MB (exe 36.6 MB) | 15 MB |
| Cold start to a visible window | median 632 ms (596 to 705) | 300 ms |
| Open `zlib-manual.pdf` to first page | median 293 ms (267 to 400) | 150 ms |

Where the time goes (debug log timestamps): `main` runs about 80 ms after process creation, and NSH's setup takes
about 500 ms more before it builds our root view (the window, its Direct3D device, DirectWrite, and the toast
notification registration that `run_app` always turns on). Our own part is small: the page canvas's device now loads
on a background thread from startup (saved about 50 ms of the first page), and the canvas then waits for NSH to show
the window before its first frame. Finding the rest needs a sampling profile of NSH's startup (WPR or samply, which
need an administrator); it is NSH's code, so a fix goes upstream like T9's proposal. Fat LTO makes a release build
take 6 to 11 minutes on this machine.

#### Ph1: 11/60 | K.25: Release engineering: installers, updates, crash reports
**P0; Phase 1; Needs:** K.1; **Code:** `packaging/`, `.github/workflows/release.yml` (Appendix D.3.9, C.3.6)
1. Windows: per-user MSI and a portable zip; signed binaries for x64 and ARM64. macOS: signed, notarized DMG.
   Linux: AppImage, then Flatpak and deb.
2. Updates: signed metadata with TUF (`tough`), stable / beta / nightly channels, staged rollout, rollback. First
   check NSH's updater (v0.5.0 replaces the app, restarts into it, and reports versions with `shellInfo()`): use it
   for download and restart if it can sit behind our TUF check, and keep `tough` for signing and rollback.
3. Opt-in crash reports: minidumps with no document content, symbolicated from a private symbol store.
4. Supply chain: reproducible builds, an SBOM per release, `cargo vet` and `cargo deny`.

**Done when:** a tagged commit produces signed installers for every desktop, and an older build updates itself.
**Progress (2026-11-14):** `packaging/windows/build-msi.ps1` builds the per-user x64 MSI (unsigned, no dialogs).
It uses only the Windows Installer API and makecab, and puts the Visual C++ runtime next to the exe (app-local).
**Links:** tough <https://github.com/awslabs/tough>; cargo-vet <https://mozilla.github.io/cargo-vet/>; minidumps
<https://github.com/rust-minidump/rust-minidump>

#### Ph0: 11/13 | K.26: Test corpus, golden images and interop suite
**P0; Phase 0 onward; Needs:** K.3; **Code:** `tests/corpus/`, `tests/golden/`, `tests/interop/`
1. Corpus: papers, books, scans, forms, CAD drawings, huge manuals, malformed files, CJK and right-to-left
   documents; the pdf.js test corpus.
2. Golden images per page and theme; compare with a perceptual threshold; review diffs.
3. Oracles: every saved file passes `qpdf --check`; `pdftotext` and `pdfsig` compare text and signatures.
4. Interop: open saved files in Acrobat Reader, PDF-XChange, Preview, pdf.js and SumatraPDF; scripted where possible,
   a manual checklist otherwise.

**Done when:** the corpus sweep (open, render, text, save) runs in CI.
**Links:** pdf.js test corpus <https://github.com/mozilla/pdf.js/tree/master/test>; Poppler utils (pdftotext,
pdfsig) <https://poppler.freedesktop.org>

#### Ph0: 12/13 | K.27: Feature checklist in CI
**P0; Phase 0; Needs:** K.1; **Code:** `docs/features.toml`, `xtask check-features` (Appendix C.3.2)
1. One row per tag: tag, title, priority, phase, status (`planned`, `in-progress`, `shipped`, `dropped`), test name,
   command id.
2. CI checks that every `shipped` row's test and command exist.
3. A generated progress report per area for each milestone (Appendix C.3.3).

**Done when:** CI fails on a `shipped` row with a missing test.

#### Ph3: 1/49 | K.28: Annotation anchors
**P0; Phase 3; Needs:** K.13, K.14; **Code:** `excellent-core::anchor` (Appendix D.1.5)
1. Each annotation gets a UUID in `/NM` and a text-quote anchor: prefix, exact text, suffix and approximate position
   (W3C Web Annotation style), kept in a private key and in the sidecar.
2. Re-anchoring: find the exact text near the old position; fall back to fuzzy matching; mark annotations that can't
   be placed as "orphaned" for the user to place.
3. Used by A.16 (transfer), B.6 (personal bookmarks), X.1 (canvas links) and Y.1 (sync).

**Done when:** annotations survive a re-typeset version of the same paper with at least 95 % placed correctly on a
test set.
**Links:** W3C Web Annotation model <https://www.w3.org/TR/annotation-model/>

### 27.2 Reading and navigation (R)

#### Ph1: 12/60 | R.1: Page layouts
**P0; Phase 1; Needs:** K.6, K.8; **Code:** `excellent-core::layout`, `excellent-features/read/layout.rs`
1. Expose the K.6 layouts as View state: single, continuous, facing, book (cover alone), horizontal, grid.
2. Toolbar toggle and commands; remember per document (in the View state).
3. Keep the reading position (page and offset) when the layout changes.

**Done when:** switching layouts keeps the same text at the top of the view; book layout puts page 1 alone.

**Built (2026-11-14), items 1 and 3:** the view state is `excellent-core::view` (K.5); it is still kept per canvas, not
per document, until R.9 and K.20 store it. Commands: Single page (Ctrl+6), Facing pages (Ctrl+7), Book view (Ctrl+8), Horizontal
strip, Page grid (columns follow the window width), Toggle continuous scrolling. `Viewport::anchor` and `relayout`
keep the reading position (`a_new_layout_keeps_the_reading_position`); `book_view_puts_the_cover_alone_on_the_right`.
Open: the toolbar toggle (no toolbar yet) and menus (K.16); remembering per document (K.5, K.20). Enhanced's plain `c`
is bound as a page key (K.16).

#### Ph1: 13/60 | R.2: Zoom and fit modes
**P0; Phase 1; Needs:** K.6, K.7; **Code:** `excellent-features/read/zoom.rs`
1. Fit page, fit width, fit height, fit content (crop to content box from the display list bounds), actual size,
   presets from 10 % to 6400 %, custom zoom dialog.
2. Ctrl+wheel and pinch zoom around the pointer; keyboard Ctrl+= / Ctrl+−; zoom snaps to buckets only for tile
   reuse, not for the displayed value.
3. Remember zoom per document.

**Done when:** zooming around the pointer keeps the point under the pointer fixed (property test).

**Built (2026-11-14), part:** `viewport::ZoomMode` (custom, fit width, fit page, fit height) stays in force through
resizes and layout changes; commands Fit page (Ctrl+0), Fit width (Ctrl+2), Fit height, Actual size (Ctrl+1); Ctrl+wheel
and Ctrl+= / Ctrl+− zoom about the pointer, and the shown zoom is not snapped (only tiles use buckets). The property
test is `zooming_keeps_the_point_under_the_pointer`. Open: fit content, the preset list, the custom zoom dialog,
pinch (U.3), remembering per document.

#### Ph1: 14/60 | R.3: Reading direction: right-to-left and manga
**P0; Phase 1; Needs:** R.1; **Code:** `excellent-core::layout`
1. A direction flag in the layout: pages ordered right to left in facing, book and horizontal layouts.
2. Arrow keys and swipe follow the direction; auto-detect from `/ViewerPreferences /Direction /R2L`.

**Done when:** a right-to-left PDF opens in the correct order without a setting.

**Built (2026-11-14), item 1 and the detection:** in `excellent-canvas::layout`, not `excellent-core::layout` (K.6).
`Direction` mirrors facing, book, grid rows and the horizontal strip (`right_to_left_mirrors_the_rows_and_the_strip`).
`Document::reads_right_to_left` reads `/ViewerPreferences /Direction /R2L` and the canvas opens such a PDF right to
left (`page_boxes_are_read_without_loading_and_match_the_page` checks the flag). Command: Toggle right-to-left reading
(manga). In the strip, the plain wheel and Page Down move toward the later pages either way. Open: arrow keys and
swipe (with U.3); EPUB's page-progression direction.

#### Ph1: 15/60 | R.4: Rotation and crop-box view
**P0; Phase 1; Needs:** K.6; **Code:** `excellent-core::transform`
1. View rotation in 90° steps (View state, not saved to the file); "Rotate pages" that changes the file is O.1.
2. Box choice: crop box (default), media box, trim box, art box, bleed box.

**Done when:** annotations and selection stay correct under every rotation and box (property test with K.6).

**Built (2026-11-14):** view rotation in quarter turns (Ctrl+Shift+= and Ctrl+Shift+−; tiles re-render rotated) and the
five boxes in the palette (the boxes are read again on a background thread). Ink is kept per page in page points and
placed through `excellent-canvas-win::placement`, so it stays on its page under every rotation, box, zoom and layout.
Tests: `page_to_device_and_back` (proptest over every rotation and offset crop boxes),
`a_point_comes_back_under_every_rotation`. Selection isn't built yet; it maps through the same placement. Enhanced's
`[` and `]` wait for keys scoped to the canvas.

#### Ph1: 16/60 | R.5: Thumbnails
**P0; Phase 1; Needs:** K.7; **Code:** `excellent-features/read/thumbnails.rs`
1. Sidebar tab with NSH `virtual_list`; thumbnails rendered at low priority from display lists; cached on disk as
   WebP keyed by document hash, page and revision.
2. Click to go; current page highlighted; page labels under each thumbnail.

**Done when:** a 5,000-page document's thumbnail list scrolls at 60 fps and never delays visible tiles.

**Built (2026-11-14), all but the measurement:** the left dock's **Pages** tab is an NSH `virtual_list`; the rows near
its viewport (6 more each way) render on the shared render pool (`excellent-core::shared`, which the canvas now uses
too) at `Priority::Thumbnail`, reusing and keeping the canvas's display lists (`excellent-core::thumbnail`). The cache
is **PNG, not WebP**: MuPDF writes PNG (`Pixmap::to_png`) and no WebP encoder is in the workspace; files are 320 pixels
wide in the app's data folder, named by the file identity's key (`FileId::key`), page, revision and width
(`excellent-features::read::thumbnails`). The canvas's `pages` event now carries page sizes so the frames never change
shape. A press goes to the page; the reader's page is ringed; labels under each. Open: the 60 fps measurement (K.24);
the list doesn't follow the reader's page yet; the cache isn't trimmed (K.20); thumbnails ignore the view's rotation.
Tests: `frames_keep_the_page_shape_and_fit_the_box`, `the_rows_shown_and_a_margin_are_wanted`,
`names_change_with_the_file_the_page_and_its_revision`, `a_page_becomes_a_png_file_of_the_asked_width`.

#### Ph1: 17/60 | R.6: Page labels and go to page
**P0; Phase 1; Needs:** K.3; **Code:** `excellent-features/read/goto.rs`
1. Read labels with `fz_page_label`; show "iv (4 of 210)" in the status bar.
2. Go-to box (Ctrl+G) accepts a label or a number; first and last page, next and previous screen.
3. Editing labels is in O.1 (`pdf_set_page_labels`, `pdf_delete_page_labels`).

**Done when:** typing "iv" in a book with roman-numbered front matter jumps to the right page.

**Built (2026-11-14):** labels are read once per open with `pdf_page_label` (shim `ex_page_label`; no page is loaded;
`Document::page_labels` gives `None` when the PDF has no `/PageLabels`). The canvas reports them to the app with its
`pages` event, and the reader's page (the one showing most) with `page`. `excellent-features::read::goto::Pages` finds
a label first (ignoring case), then a number. The reader's page shows as "iv (4 of 210)" in the **title bar**, not a
status bar (shortcut: no status bar exists yet; it moves there with the status bar), and pressing it opens the go-to
box. The box (Ctrl+G) checks the input as it is typed: where Enter goes, or that no page has that name. Commands: Go
to page… (Ctrl+G), First page (Ctrl+Home), Last page (Ctrl+End); next and previous screen came with R.2. A jump is
remembered in the history (R.8). Tests: `typing_a_roman_label_finds_its_page`,
`a_label_wins_over_a_number_and_numbers_fill_the_rest`, `page_labels_are_read_and_absent_labels_are_none`. Enhanced's
plain `g`, Home and End wait for keys scoped to the canvas, since they would fire while typing in a field.

#### Ph1: 18/60 | R.7: Links
**P0; Phase 1; Needs:** K.8; **Code:** `excellent-features/read/links.rs`
1. Load with `fz_load_links`; resolve internal targets with `fz_resolve_link`; hit-test in the canvas.
2. Internal links push history (R.8); external URLs ask for confirmation unless the protocol is allowed in settings;
   `file:` links and launch actions always ask (§14.2).
3. Hover shows the target (page preview for internal links, URL for external).

**Done when:** links work in PDF, EPUB and XPS; a `javascript:` or launch link never runs without confirmation.

**Built (2026-11-14), all but the page preview:** the shim reads a page's links and resolves each internal one to a
page and point (`excellent-mupdf::Link`, `Page::links`); the render workers read the links of the pages near the screen
and the canvas finds the one under the mouse (hand cursor). An internal link jumps to its point and pushes history
(R.8); any other goes to the app, where `excellent-features::read::links::follow` decides: protocols on the
`link_protocols` setting (default `http,https,mailto`, as Enhanced's `LinkProtocols`) open at once, other protocols
and `file:` or launch links ask in the app's own dialog (Cancel first), and `javascript:`, `vbscript:`, `data:`,
`blob:` and `about:` never run, whatever the setting says. MuPDF drops `/JavaScript` actions, so they reach the app as
links with no address and do nothing. Hover shows the target in the title bar beside the page indicator ("Go to page
iv (4 of 210)" or the address) until the status bar exists. Open: the page preview for internal links waits for
thumbnails (R.5); `link_protocols` is in the Settings window (K.16); EPUB and XPS links are read by the same
code but have no test yet. Tests: `links_are_read_and_resolved`, `web_links_open_and_other_protocols_ask`,
`scripts_never_run_and_files_always_ask`, `a_link_target_shows_at_the_top`, `file_links_become_paths`.

#### Ph1: 19/60 | R.8: Back and forward history
**P0; Phase 1; Needs:** K.5; **Code:** `excellent-core::view::history`
1. History entries (page, offset, zoom) per View, pushed on jumps (links, go to page, outline, search), not on
   scrolling.
2. Alt+← / Alt+→ and mouse back / forward buttons.

**Done when:** following three links and pressing back three times returns to the start.

**Built (2026-11-14):** `excellent-core::view::history`: a `Place` (page anchor, zoom mode and zoom)
is pushed by each jump, not by scrolling; the back and forward lists hold at most 256 places. Commands Back (Alt+←) and
Forward (Alt+→), and the mouse's back and forward buttons. Go to page, first and last page and links (R.7) push; the
outline and search push when they are built. Tests: `three_jumps_and_three_backs_return_to_the_start`,
`a_new_jump_forgets_the_places_ahead`, `history_is_bounded`.

#### Ph1: 20/60 | R.9: Session restore
**P0; Phase 1; Needs:** K.5, K.20; **Code:** `excellent-app::session`
1. Save windows, workspaces, panes, tabs and View states to `library.db` (debounced, and on quit).
2. On start, restore lazily: only the visible tab of each pane opens (as W.14).
3. Missing files show greyed out with "Locate…".

**Done when:** quitting and restarting restores every tab at the same place.

Built (2026-11-14, `docs/features/R.9-session-restore.md`) for the one window and its default workspace: windows,
panes and other workspaces come with W.1 and W.4. A tab's View state is saved as JSON holding the page only; zoom,
layout and the position within the page join it later without a migration. Missing files show greyed in the strip,
and the document panel (not the strip) offers "Locate…".

#### Ph1: 21/60 | R.10: Recent and pinned files
**P0; Phase 1; Needs:** K.20; **Code:** `excellent-features/library/recent.rs`
1. Recent list with last page and a thumbnail; pin and unpin; remove from list; clear.
2. Shown in the home page (F.17), the File menu, the jump list (Windows) and the Dock menu (macOS).

**Done when:** the recent list survives restarts and portable-mode moves.

Built (2026-11-14, `docs/features/R.10-recent-files.md`), without thumbnails, the File menu, the jump list and the
Dock menu. The code lives in `excellent-store` (`recent`, `home`) and `excellent-app::recent`, not
`excellent-features/library/recent.rs`. Until F.17, the empty document panel shows the list, and the tab list
(Ctrl+Shift+A) has a "Recent files" section. Portable moves: the library records the app's folder in a `meta` table.
When the folder changes, it rewrites the paths inside the old folder, and paths on the old drive when the drive letter
changed and the file is found under the new letter.

#### Ph1: 22/60 | R.11: Opening files: drag and drop, file associations, single instance, command line
**P0; Phase 1; Needs:** K.19; **Code:** `excellent-app::open`, `crates/excellent-platform`
1. Drop files on the window or a pane edge (V.2). NSH v0.5.0 reports each dropped file's MIME type; accept a drop by
   type and by sniffing the file's first bytes, not by extension alone.
2. Register file types per user on install; "Set as default viewer" opens the OS settings page.
3. Single instance: a second launch sends its arguments to the running instance over a named pipe or socket.
4. Command line: files, `--page`, `--zoom`, `--search`, `--split` (V.13), `--forward-search` (R.27).

**Done when:** double-clicking a PDF in the file manager opens it as a tab in the running instance.

Built (2026-11-14, `docs/features/R.11-opening-files.md`) on Windows. Drops open documents in new tabs. A drop is
accepted by the file's first bytes; NSH's MIME types come from the file name, so they aren't used. Step 2 is the
"Set as default PDF viewer" command for now: it registers the ProgID, "Open with" entries and Capabilities for the
user, then opens Default apps, because Windows lets no app make itself the default. Registering on install waits for
the installer. Step 3 uses a per-user named pipe. Step 4 reads files and `--page`; `--zoom`, `--search`, `--split`
and `--forward-search` wait for their features. K.19 is built only as far as R.11 needs (`instance`,
`associations`), without the traits and the test fake yet.

#### Ph1: 23/60 | R.12: Tabs
**P0; Phase 1; Needs:** K.5; **Code:** `excellent-app::tabs`
1. NSH `Tabs` with reorder, pin, close, close others, close to the right, and a dirty dot.
2. Ctrl+Tab and Ctrl+Shift+Tab (most-recent order, setting), Ctrl+W, Ctrl+Shift+T reopen closed tab (per workspace,
   W.4).
3. Tab search (Ctrl+Shift+A) across workspaces (W.9).
4. The tab-list button at the right end of the tab strip (§10.1; reference image
   `dev notes/images/ui-reference-vivaldi-tab-list.png`): a chevron opening a dropdown with "Search all tabs" (the
   same search as step 3) and collapsible, counted sections for open, inactive (unloaded, W.14), duplicate (same file
   in more than one tab, "N copies", workspace badge, "Close all duplicate tabs") and recently closed tabs (step 2's
   list). Each row activates, closes or reopens its tab; the whole list works from the keyboard.

**Done when:** 50 tabs open with only visible ones loaded (W.14); the tab-list dropdown finds any of them by name.

Built (2026-11-14), steps 1, 2 and 4 without workspaces (`docs/features/R.12-tabs.md`). Changes from the steps above:
NSH v0.5.0's `Tabs` has no close or reorder, so each tab is an NSH `Tab` with a close button of its own, moved with
Ctrl+Shift+PageUp / PageDown (mouse dragging still to come); the tab list is a dialog like the palette, not a
dropdown, because the page canvas is a window over the app's and a popup under the title bar would be drawn behind
it (the same holds for a tab's context menu, so its commands are in the palette); "inactive" tabs are every tab but
the shown one, since only the shown tab has a document open (one canvas, given the shown tab's file). Opening files
uses NSH's `pick_paths` (the shell's `IFileOpenDialog`), so K.19 needs no dialog of its own for this.

#### Ph1: 24/60 | R.13: Named tab groups
**P1; Phase 1; Needs:** R.12; **Code:** `excellent-app::tabs::groups`
1. Group tabs under a coloured, named label in the tab strip; collapse and expand; move tabs in and out (port the
   ideas of Enhanced's `src/TabGroupsManage.cpp`).
2. Groups are saved in sessions and workspaces.

**Done when:** groups survive restart; collapsing a group hides its tabs without unloading unsaved work.

Built (2026-11-14, `docs/features/R.13-tab-groups.md`). Enhanced's `TabGroupsManage.cpp` saves and reopens named
sets of tabs; here a group lives in the strip as browsers' do (a coloured chip that collapses its tabs), and saving
sets for later waits for workspaces (W.1). The group's menu is a dialog rather than a popup (the page canvas would
cover a popup, as with R.12's tab list), and tabs move in and out of groups with Ctrl+Shift+PageUp / PageDown until
mouse dragging exists: beside an open group a tab joins it, at its group's edge it leaves, and it passes a collapsed
group whole.

#### Ph1: 25/60 | R.14: Smart dark mode
**P0; Phase 1; Needs:** K.7; **Code:** `excellent-core::darkmode`
1. Port Enhanced's `src/PdfDarkMode*.cpp`: an `fz_device` wrapper mapping text and vector colors through OKLab to
   the theme, an image classifier (photo, light-background artwork, icon or line art, full-page scan) and per-image
   preserve, adapt or recolor (§7.5).
2. Port its unit tests (`PdfDarkMode*_ut.cpp`) first and keep them passing.
3. The dark-mode profile hash is part of the tile cache key (K.7).

**Done when:** the ported tests pass; golden images in dark mode match Enhanced's output on the corpus.

Built (2026-11-14), first step: Enhanced's default shipped path rather than step 1's object-level device. Enhanced
renders the page as usual, then maps the tile's pixels from black-on-white to the theme's ink and paper, keeping one
region per page that holds a photo or artwork (its legacy renderer with `PreserveImages`, which "smart" uses while
the object-level renderer is off by default). The image rules and thresholds are ported as they are
(`PdfDarkModeImageStats.cpp`, `BuildPageDarkLegacySkipRects`); the shim lists a page's images with a derived device.
Step 2's tests belong to the object-level device, so they wait for it. Step 3 is done differently: the colours are
not in the tile key; changing them clears the tiles, as Enhanced's epoch does. The setting is `document_colors`
(`off`, `smart`, `all`) in the Settings window's Reading group; the command `view.document_colors` (Shift+I on the
pages) switches off and smart and keeps the choice, where Enhanced's Shift+I inverts for the session only. Not built
yet: the object-level device, golden images, recoloured thumbnails.

#### Ph1: 26/60 | R.15: Reading themes
**P0; Phase 1; Needs:** R.14, K.17; **Code:** `excellent-core::darkmode::themes`
1. Light, dark, sepia, OLED black, high contrast, custom (paper and ink colors); per-document override.
2. Reflowable formats get theme CSS through MuPDF's user stylesheet instead of recoloring.
3. Live preview, as K.17's pickers: the highlighted reading theme is applied to the open page while the user moves
   through the list; Enter keeps it, Escape goes back (maintainer, 2026-11-13).

**Done when:** each theme renders the corpus with WCAG AA text contrast.

Built (2026-11-14), steps 1 and 3: themes App theme (the default, R.14's colours), Light, Sepia, Dark, OLED black,
High contrast and Custom (`reading_theme`, with `reading_ink` and `reading_paper`), in `excellent-core::darkmode::themes`.
Step 3's live preview cannot be a dialog in the middle of the window, as K.17's pickers are: the page canvas is a
window over the app's and hides under dialogs. The reading-theme bar therefore takes the palette button's place in
the title bar, outside the canvas; the arrow keys preview, Enter keeps, Escape goes back. Choosing a theme while page
colours are off switches them to smart. Not built yet: the per-document override (it needs K.20) and step 2 (EPUB
and other reflowable formats are recoloured like PDFs meanwhile).

#### Ph5: 1/45 | R.16: Reflow (Liquid Mode)
**P1; Phase 5; Needs:** K.13; **Code:** `excellent-features/reflow`
1. Build a reflow model per page from structured text with `FZ_STEXT_SEGMENT`, `FZ_STEXT_TABLE_HUNT`,
   `FZ_STEXT_PRESERVE_IMAGES` and `FZ_STEXT_COLLECT_STRUCTURE`: headings (from font size and weight), paragraphs,
   lists, tables, figures with captions, in reading order (K.13).
2. Emit simple HTML; lay it out with MuPDF's story engine (`fz_new_story`, `fz_place_story`) or NSH `TextView`.
3. Typography controls: font, size, line spacing, margins, theme.
4. Map every reflowed paragraph back to its page quads, so selection, highlights and search work in both views.

**Done when:** on a set of 50 papers, reading order is correct on 95 % of pages; a highlight made in reflow shows on
the original page.

#### Ph3: 2/49 | R.17: Mini-map with note ticks
**P1; Phase 3; Needs:** R.5, K.14; **Code:** `excellent-features/read/minimap.rs`
1. A narrow strip beside the scrollbar: one tick per page, coloured marks for highlights, notes, bookmarks and search
   hits.
2. Click or drag to scroll.

**Done when:** marks update within one frame of an annotation change.

**Built (2026-11-15):** `read::minimap` (scale, lanes, palette), `annot::page_marks`, and the canvas's strip on the
right edge (12 px, always vertical, the whole document top to bottom). Operations now know their pages, and apply,
undo and redo bump those pages' revisions; the strip reads a page again in the frame after its revision moves.
Bookmarks come from the library through a `bookmarks` prop. Not tried by hand.

#### Ph7: 1/32 | R.18: Thumbnail drag reordering
**P1; Phase 7; Needs:** R.5, O.1; **Code:** uses O.1's page operations
1. Drag thumbnails (multi-select) to reorder; drop between pages; the move is one undoable `pdf_rearrange_pages`
   operation (O.1).

**Done when:** reordering 100 pages and undoing restores the original order and the outline (§11.14, step 5).

#### Ph1: 27/60 | R.19: Full-screen reading and focus mode
**P1; Phase 1; Needs:** R.1; **Code:** `excellent-features/read/focus.rs`
1. Full screen (F11): no chrome; controls on pointer at the edge.
2. Focus mode: dims everything outside the current paragraph or line (using K.13 blocks).

**Done when:** Esc leaves both modes; focus mode follows the keyboard caret.

**Built (2026-11-14):** `read::focus` (scope, lit box, top-edge reveal), the canvas's dim (`Gfx::dim_outside`, in the
page-area colour rather than Enhanced's black, so it suits dark themes) and `excellent-app::full_screen`. F11 is the
platform's own; `Ctrl+Shift+L` (Enhanced's) toggles too, and the app follows the window's mode either way. The canvas
is a child window, so the revealed title bar pushes the pages down instead of lying over them. Focus mode follows the
reading line (R.20's arrows); a text caret does not exist yet. Settings: `focus_scope` (`"paragraph"`, `"line"`).

#### Ph1: 28/60 | R.20: Reading ruler (reading bar)
**P1; Phase 1; Needs:** K.8, K.13; **Code:** `excellent-features/read/ruler.rs`
1. Port Enhanced's reading bar (`src/ReadingBar.cpp`): a horizontal band drawn in the canvas that follows the
   pointer or moves line by line with the keyboard, snapping to K.13 text lines.
2. Height, colour and opacity settings.

**Done when:** arrow keys move the ruler exactly one text line in two-column papers.

**Built (2026-11-14):** `read::ruler`, drawn by the canvas (`canvas::reading`). It snaps to K.13 lines and steps in
MuPDF's reading order; on a page without text it is a free band. The band stays horizontal on rotated pages. Plain
keys (the arrows, Space, `+`, `−`, Esc) are scoped to the pages (`keymap::Scope::Page`): they work while the
view around the canvas has the keyboard focus, which it takes when a document opens and when the pages are pressed.
Settings: `reading_ruler_height`, `reading_ruler_color`, `reading_ruler_opacity`, `reading_ruler_invert`, in the
Settings window's Reading section (K.16).

#### Ph1: 29/60 | R.21: Auto-scroll
**P1; Phase 1; Needs:** K.7; **Code:** `excellent-features/read/autoscroll.rs`
1. Port Enhanced's `src/ReadingAutoScroll.cpp`: continuous scroll at an adjustable speed; space pauses; +/− change
   speed; middle-click auto-scroll.
2. Drive scrolling from the display's frame clock, not a timer.

**Done when:** scrolling is smooth at 60 and 120 Hz and uses no CPU while paused.

**Built (2026-11-14):** `read::autoscroll`, driven by the canvas's paint: while it moves, each frame asks for the next,
and the swap chain's wait paces them at the display's rate; paused, nothing runs. `Ctrl+Shift+H` toggles (Enhanced's);
`+` and `−` change the speed only while it runs. Needs K.7, which is not marked in `docs/features.toml`; the canvas's
current tile path is what it scrolls. Setting: `auto_scroll_speed`.

#### Ph5: 2/45 | R.22: Bionic reading
**P2; Phase 5; Needs:** R.16; **Code:** `excellent-features/reflow/bionic.rs`
1. In reflow only: bold the first part of each word (about 40 %); toggle and strength setting.

**Done when:** works for Latin, Cyrillic and Greek; turns itself off for scripts where it makes no sense.

#### Ph4: 2/44 | R.23: Paper textures
**P2; Phase 4; Needs:** R.15; **Code:** `excellent-core::darkmode::paper`
1. Subtle procedural textures (paper grain) drawn under the page in the canvas, not baked into tiles.

**Done when:** no measurable effect on scroll frame time.

*Built (2026-11-25):* `excellent-core::darkmode::paper` (a seamless 512-pixel grain) drawn by the canvas over each page
at 6 % through `Host::faint_image`, after the tiles, never in them; "Paper texture" in Settings, Reading. Frame time
not measured yet.

#### Ph5: 3/45 | R.24: Two-column auto-detect
**P2; Phase 5; Needs:** K.13; **Code:** `excellent-features/read/columns.rs`
1. Detect column layout from K.13 blocks; "column mode" scrolls down one column, then jumps to the top of the next.

**Done when:** works on the two-column papers in the corpus.

#### Ph3: 3/49 | R.25: Citation and equation hover
**P0 (parity); Phase 3; Needs:** K.13, R.7; **Code:** `excellent-features/read/refhover`
1. Port Enhanced's reference hover (`src/RefHover*.cpp`): detect citations (`[12]`, `(Smith et al., 2020)`), figure,
   table and equation references near the pointer.
2. Resolve the target: the link destination at the point if there is one, otherwise search the references section,
   captions or equation numbers.
3. Show a popover drawn in the canvas with the target region rendered.
4. Port the detector tests (`src/base/tests/RefHover_ut.cpp`) first.

**Done when:** the ported tests pass; hovering `[12]` in a paper without links shows reference 12.

#### Ph7: 2/32 | R.26: CAD drawing view
**P1 (parity); Phase 7; Needs:** R.14; **Code:** `excellent-core::cad`
1. Port Enhanced's `src/PdfCad.cpp`: scoring-based detection of CAD drawings and its colour mapping (`CadMapColor`)
   as an `fz_device` wrapper, like dark mode.
2. Options: dark background, line weights on or off, layer panel (OCGs with `pdf_count_layers`, `pdf_enable_layer`).

**Done when:** CAD drawings in the corpus are detected, and none of the non-CAD files are.

#### Ph5: 4/45 | R.27: SyncTeX forward and inverse search
**P1 (parity); Phase 5; Needs:** R.11; **Code:** `excellent-features/synctex`
1. Parse `.synctex(.gz)` with the SyncTeX parser (C, from the SyncTeX project) behind a small shim, or port Enhanced's
   `src/PdfSync.cpp`.
2. Forward search: `excellentpdf --forward-search file.pdf source.tex line` (via the single instance) highlights the
   spot.
3. Inverse search: double-click runs the editor command with `%f` and `%l`, configured in settings, with presets for
   VS Code, Vim, Emacs and TeXstudio.
4. Reload automatically when LaTeX rewrites the PDF (K.15).

**Done when:** VS Code's LaTeX Workshop can use ExcellentPDF as external viewer in both directions.
**Links:** SyncTeX <https://github.com/jlaurens/synctex>

#### Ph1: 30/60 | R.28: Other formats through MuPDF
**P0; Phase 1; Needs:** K.3; **Code:** `excellent-core::formats`
1. Open EPUB, MOBI, FB2, XPS, CBZ, SVG, images, HTML and Markdown with `fz_open_document`.
2. Reflowable formats: `fz_is_document_reflowable` and `fz_layout_document` with the page size and font size from the
   View; re-layout on window resize, keeping the position by text location.

**Done when:** every format in the corpus opens; font size changes keep the reading position in EPUB.

Built (2026-11-14, `docs/features/R.28-other-formats.md`). The format list and the signatures live in
`excellent-core::formats`, shared by the Open dialog, drops and file associations; DjVu left the list (it is R.29's,
and MuPDF has no DjVu reader). Step 2: a book's page is the window's size at zoom 1, up to 36 ems wide, at the "Book
text size" setting (8 to 32 pt, 12 by default); the position is kept with MuPDF's bookmarks (`fz_make_bookmark` before
`fz_layout_document`, `fz_lookup_bookmark` after). Two views of one book share one layout until V.1.

#### Ph5: 5/45 | R.29: DjVu, CHM and CBR
**P2; Phase 5 (if Q5 says yes); Needs:** R.28; **Code:** separate engine crates behind the `Document` trait
1. One crate per engine: DjVu (DjVuLibre), CHM (chmlib), CBR (UnRAR). Check each licence first; UnRAR forbids
   re-creating the RAR compression algorithm.
2. Same `Document` trait as MuPDF; text where the format has it.

**Done when:** the files Enhanced opens in these formats open here.

#### Ph5: 6/45 | R.30: Open from a URL with progressive loading
**P1; Phase 5; Needs:** K.3, K.22; **Code:** `excellent-core::remote` (Appendix D.4)
1. A custom `fz_stream` (`fz_new_stream`) with `progressive = 1` over an HTTP range-request reader with a block cache.
2. MuPDF throws `FZ_ERROR_TRYLATER` when data is missing: show a placeholder, fetch the range, retry.
   Linearized files (`pdf_doc_was_linearized`) show page 1 first.
3. Servers without range support: download fully with progress.
4. "Save a copy" writes the downloaded bytes.

**Done when:** an arXiv PDF shows page 1 before the download finishes.
**Links:** HTTP range requests <https://developer.mozilla.org/en-US/docs/Web/HTTP/Range_requests>

#### Ph6: 1/13 | R.31: Visual search (Google Lens)
**P2 (parity); Phase 6; Needs:** O.23; **Code:** `excellent-features/lens`
1. Select a region (O.23), confirm the upload, then send the rendered PNG to Google Lens in the browser, following
   Enhanced's `src/GoogleLens.cpp`.
2. Disabled when online features are off.

**Done when:** a region opens in Lens after one confirmation; nothing is sent without it.

### 27.3 Bookmarks and outline (B)

The design is in §11.14 ("How it works", steps 1–7). The plans below are the build order.

#### Ph1: 31/60 | B.1: Add an outline entry
**P0; Phase 1; Needs:** K.3, K.13; **Code:** `excellent-core::outline`, `excellent-features/outline`
1. Outline model with stable ids mapped to iterator paths (§11.14, step 4).
2. Add at the current view: `fz_outline_iterator_insert` with a URI from `fz_format_link_uri` (page, XYZ, zoom).
3. Add from selection: title is the selected text, destination the top of the selection.
4. No outline yet: the first add creates one.

**Done when:** an added entry opens at the same place in Acrobat and pdf.js.

**Built (2026-11-14), the reading part:** the sidebar's **Outline** tab shows the outline as an NSH `Tree`, read on
the render workers when the tab first shows (`excellent-core::outline::load`). Ids are paths from the top ("2.1.4"),
the iterator paths of step 1 (`excellent-features::outline`). A press or the arrows go to the entry's page (through go
to page, so Back returns); the reader's entry (the nearest page at or before theirs, the last in document order; a
chosen entry stays while it opens that page) is selected and kept in view; entries start expanded as the file says.
Tests: `ids_are_paths_and_find_their_entries`, `the_reader_is_in_the_nearest_entry_before_the_page`,
`a_chosen_entry_stays_while_it_opens_the_page`, `an_outline_is_read_on_the_workers`.

**Built (2026-11-14), steps 2 to 4:** **Add outline entry here** (Ctrl+B, Enhanced's CmdAddBookmark) inserts after
the selected entry. The canvas answers `outlineTarget` with the top of the selection, or the top-left of the view,
and the zoom only when it is a custom one; the title is the selected text, else "Page n" selected in place to type
over. A document without an outline offers "Add an entry for this page". Every edit reads the outline from the
document on the UI thread first, so paths are never stale (a shortcut for very large outlines). Open: checking an
added entry in Acrobat and pdf.js, hover preview, the 10,000-entry check. Tests: `paths_titles_and_where_a_new_entry_goes`,
`the_first_entry_makes_an_outline`.

#### Ph1: 32/60 | B.2: Rename and delete
**P0; Phase 1; Needs:** B.1; **Code:** as B.1
1. F2 inline rename through `fz_outline_iterator_update`.
2. Delete with children (`fz_outline_iterator_delete`), or promote children one level first.

**Done when:** undo restores the exact previous outline.

**Built (2026-11-14):** F2 edits the title in place (Return or leaving keeps it, Escape drops it); Delete takes the
entry with its children, the last first so paths stay right. Both in the row's context menu and the palette. Open:
promoting the children first. Tests: `deletes_go_from_the_last_and_skip_entries_already_going`,
`outline_edits_are_undone_and_redone_one_operation_at_a_time`.

#### Ph1: 33/60 | B.3: Move entries
**P0; Phase 1; Needs:** B.1; **Code:** as B.1
1. Move = read the subtree, delete, walk to the target, insert again (§11.14, step 2), as one journal operation.
2. Drag and drop in NSH `Tree` (before, after, into); Alt+↑ / Alt+↓, Tab / Shift+Tab.

**Done when:** moving a subtree of 500 entries deep in a 10,000-entry outline keeps every child.

**Built (2026-11-14), the keys:** Alt+↑ / Alt+↓, Tab / Shift+Tab while the outline has the keyboard (so Tab no
longer leaves the tree; Escape gives the keyboard to the pages), also in the context menu and palette. `plan_shift`
plans the move on a copy of the tree. Not possible yet: drag and drop, since NSH v0.5.0's `Tree` has none and NSH
has no element drag. Open: the 500-entry check. Tests: `entries_move_one_place_as_a_group`,
`entries_go_in_and_out_with_the_entries_under_them`.

#### Ph1: 34/60 | B.4: Set destination to the current view
**P0; Phase 1; Needs:** B.1; **Code:** as B.1
1. Rewrite the entry's URI from the current View; show page and zoom in the inspector.

**Done when:** the repointed entry opens at the new place after save and reopen.

**Built (2026-11-14):** **Point outline entry here** (context menu, palette) takes the place as B.1 does. Open: the
inspector's page and zoom. Test: `a_repointed_entry_opens_its_new_place_after_reopening`.

#### Ph1: 35/60 | B.5: Multi-select, undo and open state
**P0; Phase 1; Needs:** B.2, B.3; **Code:** as B.1
1. Multi-select for delete, move and restyle; each action is one operation.
2. Save open and closed state (`is_open`) on change.

**Done when:** a multi-entry move undoes in one step.

**Built (2026-11-14):** Ctrl+press marks, Shift+press marks a run; delete and moves act on the marked entries as one
operation. Open state is written only inside an edit, for entries whose shown state differs from the file's, so
expanding alone never makes the file unsaved (a change to step 2). Open: restyle; an end-to-end check of the one-step
undo. Tests: `entries_move_one_place_as_a_group`, `only_open_state_the_reader_changed_is_written`.

#### Ph1: 36/60 | B.6: Personal bookmarks
**P0; Phase 1; Needs:** K.20, K.28; **Code:** `excellent-features/bookmarks`
1. Table `bookmark`: id, file id, anchor (page, offset, zoom, text anchor), title, note, color, tags, created.
2. Ctrl+D adds; own sidebar tab with search; jump; edit note, color and tags.
3. Works in every format; re-anchors by text when the file changes (K.28).

**Done when:** bookmarks survive a file rename (found by file identity) and a re-typeset version (K.28).

**Built (2026-11-14), steps 1 and 2; step 3 in part:** model in `excellent-features::bookmarks` (tags, search), table
in `excellent-store::bookmarks` (migration 4), view in `excellent-app::bookmarks`. The anchor is page, point and zoom
plus the selected text (`quote`); tags are a table of their own; colour reuses the tab groups' palette. A rename or
move is found by content (`FileId::content_key`, stored as `file.content`) when the file is next shown. The canvas's
`outlineTarget` is now `readerPlace`, and `goToPoint` jumps to a point. Open: re-anchoring by text (K.28, Phase 3),
so a re-typeset version, and a file edited and then renamed before it is shown, don't find their bookmarks yet.

#### Ph3: 4/49 | B.7: Bold, italic and color
**P1; Phase 3; Needs:** B.1; **Code:** as B.1
1. Set `flags` and `r g b` through `fz_outline_iterator_update`; draw with theme-aware colors as Enhanced does.

**Done when:** styles show the same in Acrobat.

#### Ph5: 7/45 | B.8: Generate an outline
**P1; Phase 5; Needs:** B.1, K.13; **Code:** `excellent-features/outline/generate.rs`
1. From headings: font size and weight in structured text, plus Enhanced's numbering rules ("1.2.", "II.A.").
2. From structure tags, when the PDF is tagged.
3. One entry per file when combining (O.2).
4. Show a preview tree to accept, edit or discard.

**Done when:** on 20 untagged books the generated outline matches the printed table of contents for 90 % of
chapters.

#### Ph3: 5/49 | B.9: Check destinations
**P1; Phase 3; Needs:** B.1; **Code:** as B.1
1. Resolve each entry (`fz_resolve_link_dest`); flag missing pages and empty targets; "fix" repoints to the nearest
   heading or removes the entry.

**Done when:** every broken entry in a test file is found.

#### Ph3: 6/49 | B.10: Sort, promote and demote between outline and bookmarks
**P1; Phase 3; Needs:** B.1, B.6; **Code:** as B.1
1. Sort children by page.
2. Promote a personal bookmark into the outline, and turn an outline entry into a personal bookmark.

**Done when:** both directions keep title and destination.

#### Ph5: 8/45 | B.11: Import and export
**P1; Phase 5; Needs:** B.1; **Code:** `excellent-features/outline/io.rs`
1. Indented text, JSON and pdftk's `BookmarkBegin` format; copy the outline as Markdown.

**Done when:** export then import round-trips exactly.
**Links:** pdftk dump_data format <https://www.pdflabs.com/docs/pdftk-man-page/>

#### Ph7: 3/32 | B.12: Other outline actions
**P2; Phase 7; Needs:** B.1; **Code:** as B.1
1. Show and edit link, file and named actions in the inspector, with an icon in the tree.

**Done when:** an outline entry pointing to another file opens that file at the right page.

#### Ph5: 9/45 | B.13: Custom outline for non-PDF formats
**P2; Phase 5; Needs:** B.6; **Code:** sidecar (§9.2)
1. Keep an editable outline in the sidecar for EPUB, CBZ and other read-only formats; shown instead of the built-in one
   when present.

**Done when:** a custom outline on a CBZ survives restart.

### 27.4 Split view (V)

The design is in §11.15 ("How it works", steps 1–8). It depends on K.5 (Document and View).

#### Ph1: 37/60 | V.1: Split the current tab
**P0; Phase 1; Needs:** K.5, K.8; **Code:** `excellent-app::panes`
1. Ctrl+\ (right) and Ctrl+Alt+\ (down) create a new View of the same Document in a new pane, at the same place.
2. Each pane gets its own canvas instance; tiles are shared through the Document's cache.

**Done when:** the second pane's first frame appears within 50 ms with no re-parse.

#### Ph1: 38/60 | V.2: Open another file in a pane
**P0; Phase 1; Needs:** V.1, R.11; **Code:** as V.1
1. "Open in split" in the file dialog, recent files and library; drop targets on the four edges of the page area.

*Built (2026-11-15):* the open dialog (Ctrl+Shift+O) and the edge drop targets; recent files and the library follow
with F.1.

**Done when:** dropping a file on the right edge opens it in a new right pane.

#### Ph1: 39/60 | V.3: Tab groups per pane
**P0; Phase 1; Needs:** V.1, R.12; **Code:** as V.1
1. NSH `DockArea` groups (or `ResizablePanelGroup` plus `Tabs`); drag tabs between groups; "Move to other group".

*Built (2026-11-15):* a `ResizablePanelGroup` grid with each pane's own tabs (the title bar's strip is the active
pane's); "Move tab to the next / previous pane" (Ctrl+Alt+Right / Left). Mouse drag between panes comes with R.12's
tab drag.

**Done when:** moving a tab keeps its View state.

#### Ph1: 40/60 | V.4: Pane layout
**P0; Phase 1 (2 panes), P1 (grid of 4); Needs:** V.3; **Code:** as V.1
1. Side by side, stacked, and a grid of up to four; divider drag; double-click divider to equalize.

**Done when:** layouts are plain data that round-trip through JSON.

#### Ph1: 41/60 | V.5: Active pane and focus
**P0; Phase 1; Needs:** V.1; **Code:** as V.1
1. Focus ring in the mode accent; Ctrl+1…Ctrl+4 focus panes; every command acts on the active pane.

*Proposed change (dev_decisions DD4, waiting for the maintainer):* Ctrl+1…Ctrl+9 already go to tabs (R.12, as
Enhanced and the browsers), so panes take Alt+1…Alt+4, plus F6 / Shift+F6 for the next and previous pane.

**Done when:** keyboard-only users can reach every pane.

#### Ph1: 42/60 | V.6: Closing panes
**P0; Phase 1; Needs:** V.3; **Code:** as V.1
1. Closing a pane moves its tabs to the neighbour or closes them (setting); closing the last View of a dirty
   Document prompts.

*Built (2026-11-15):* the setting is `close_pane_tabs` (`"move"`, the default, or `"close"`). No prompt: K.15 saves
edits 2 s after the last one and whenever a pane stops showing a file, so nothing unsaved is left to ask about.

**Done when:** closing panes in any order never loses unsaved work (test).

#### Ph1: 43/60 | V.7: Saving split layouts
**P0; Phase 1; Needs:** V.4, R.9; **Code:** as V.1
1. Sessions and workspaces store the layout, each pane's tabs and each View.

**Done when:** a restart restores the exact split.

#### Ph3: 7/49 | V.8: Synchronized scrolling
**P1; Phase 3; Needs:** V.1; **Code:** `excellent-app::panes::sync`
1. Modes: same page, same relative position, matched page pairs (user marks pairs); zoom sync separate.
2. Ctrl+Alt+S toggles; off by default.

**Done when:** sync stays aligned across documents with different page sizes.

#### Ph3: 8/49 | V.9: Open a link in the other pane
**P1; Phase 3; Needs:** V.1, R.7; **Code:** as V.1
1. Ctrl+click or middle-click on a link, footnote, citation, outline entry or search result opens the target in the
   other pane (creating one if needed).

**Done when:** the reading position in the source pane doesn't move.

#### Ph3: 9/49 | V.10: Swap, flip and maximize
**P1; Phase 3; Needs:** V.4; **Code:** as V.1
1. Swap panes; flip side-by-side ↔ stacked; maximize one pane (double-click its tab) and restore.

**Done when:** all three are undoable layout changes.

#### Ph3: 10/49 | V.11: Pop out to a window
**P1; Phase 3; Needs:** V.3; **Code:** `excellent-app::windows`
1. Move a tab or group into a new window opened with NSH's `open_window` (v0.5.0); keep its `WindowRef` in the
   window registry; drag the tab back to dock it. All windows share one UI thread, and NSH paces each one's frames.
   *(Built: the shown tab moves; it docks with the same command, the window's Dock button or closing the window,
   since NSH 0.5 has no drag between windows. Groups and drag to dock wait for that.)*

**Done when:** popped-out tabs keep their View state and share the Document.

#### Ph5: 10/45 | V.12: Tiling selected tabs
**P2; Phase 5; Needs:** V.4; **Code:** as V.1
1. Multi-select tabs → "Tile" into a grid; "Untile" returns them to one group.

**Done when:** tile and untile round-trip.

#### Ph1: 44/60 | V.13: Split from the command line
**P1; Phase 1; Needs:** V.2, R.11; **Code:** `excellent-app::open`
1. `excellentpdf --split a.pdf b.pdf` and `--split-same a.pdf`.

**Done when:** both work from a cold start and through the single instance.

#### Ph8: 2/3 | V.14: Two panes on tablets
**P1; Phase 8; Needs:** V.1, K.11, K.12; **Code:** as V.1
1. Landscape tablets allow two panes and V.9; phones stay single-pane.

**Done when:** works on iPad and an Android tablet with stylus input going to the pane under the pen.

### 27.5 Workspaces (W)

The design is in §11.16 ("How it stays efficient", steps 1–8, and its storage tables).

#### Ph1: 45/60 | W.1: Workspace switcher
**P0; Phase 1; Needs:** W.14; **Code:** `excellent-app::workspaces`
1. Button at the left of the tab bar (icon and name); popover listing workspaces with color, tab count and dirty dot;
   also in the left rail and the palette.

*Built (2026-11-15):* the button and the palette; the list is a dialog (a popover falls behind the page canvas).
No dirty dot: K.15 saves edits within 2 s. The left rail entry comes with the rail.

**Done when:** switching by mouse, keyboard and palette all work.

#### Ph1: 46/60 | W.2: Create a workspace
**P0; Phase 1; Needs:** W.1; **Code:** as W.1
1. Empty, from all tabs, or from selected tabs ("Move to new workspace").

*Built (2026-11-15):* "from selected tabs" is the shown tab until R.12 has multi-select.

**Done when:** each way creates the expected tabs.

#### Ph1: 47/60 | W.3: Edit and delete workspaces
**P0; Phase 1; Needs:** W.1; **Code:** as W.1
1. Rename, recolor, change icon, reorder by drag; delete asks to close its tabs or move them.

*Built (2026-11-15):* reorder with Move up / Move down (dialog and commands); drag comes later. Deleting needs no
dirty prompt: K.15 has saved every edit.

**Done when:** deleting never closes a dirty Document without a prompt.

#### Ph1: 48/60 | W.4: Move or copy tabs between workspaces
**P0; Phase 1; Needs:** W.1; **Code:** as W.1
1. Tab context menu, or drag a tab onto the switcher; closed-tab lists are per workspace.

*Built (2026-11-15):* "Move tab to workspace" and "Copy tab to workspace" (palette and the switcher's list); the
context menu and the drag wait for popups over the canvas and R.12's tab drag.

**Done when:** a moved tab keeps its View state.

#### Ph1: 49/60 | W.5: Workspace shortcuts
**P0; Phase 1; Needs:** W.1, K.16; **Code:** as W.1
1. Ctrl+Alt+1…9; Ctrl+Alt+PgUp / PgDn.

**Done when:** shortcuts are rebindable (K.16).

#### Ph1: 50/60 | W.6: Opening a file that is open elsewhere
**P0; Phase 1; Needs:** W.1, K.5; **Code:** as W.1
1. New files open in the current workspace; if already open in another, the setting picks "switch there" (default)
   or "open here as well" (shares the Document through the registry).

**Done when:** both settings behave as described.

#### Ph1: 51/60 | W.7: Restore per window
**P0; Phase 1; Needs:** R.9; **Code:** as W.1
1. Each window reopens on its last workspace.

*Built (2026-11-15):* one window, so one remembered workspace (`meta`); W.8 keys it per window.

**Done when:** two windows restore two workspaces.

#### Ph3: 11/49 | W.8: One workspace per window
**P1; Phase 3; Needs:** W.7; **Code:** as W.1
1. "Open workspace in new window" (`open_window`, NSH v0.5.0); a workspace open in one window can't be opened in
   another (focus that window through its `WindowRef` instead).

*Built (2026-11-16):* the rule is `workspaces::claims`, asked by every way a window comes to show a workspace; the
other windows' workspaces are kept in `meta` for the next start, which also completes W.7.

**Done when:** the rule holds under every open path (test).

#### Ph3: 12/49 | W.9: Search tabs across workspaces
**P1; Phase 3; Needs:** R.12; **Code:** as W.1
1. Ctrl+Shift+A lists tabs from every workspace, grouped; Enter switches and focuses.

**Done when:** 5,000 tabs filter as you type within 16 ms.

*Built (2026-11-16):* Ctrl+Shift+A is R.12's tab list, with a section per other workspace; Enter in one goes through
W.8's rule, so a workspace another window shows brings that window forward.

#### Ph5: 11/45 | W.10: Workspaces and library collections
**P1; Phase 5; Needs:** F.6; **Code:** as W.1
1. "Open collection as workspace" and "Save workspace as collection".

**Done when:** both directions keep file order.

#### Ph3: 13/49 | W.11: Export and import
**P1; Phase 3; Needs:** W.1; **Code:** as W.1
1. JSON with paths relative to a chosen root where possible; missing files show greyed out with "Locate…".

**Done when:** export then import round-trips exactly.

*Built (2026-11-16):* the chosen root is the folder the workspace file is saved in (picking where to save is picking
the root); paths under it are relative, others absolute. Closed tabs aren't exported. Missing files use R.9's greyed
tab and "Locate…".

#### Ph5: 12/45 | W.12: Templates and rules
**P2; Phase 5; Needs:** W.2; **Code:** as W.1
1. Templates (for example "Course": syllabus, textbook, notebook); rules ("files from this folder open in workspace
   X").

**Done when:** a rule routes a newly opened file correctly.

#### Ph9: 1/13 | W.13: Sync workspaces
**P2; Phase 9; Needs:** Y.1; **Code:** as W.1
1. Sync the three workspace tables through folder sync, with paths mapped per device.

**Done when:** a workspace created on one device appears on another.

#### Ph1: 52/60 | W.14: Lazy restore and hibernation
**P0; Phase 1; Needs:** K.5, K.20; **Code:** `excellent-app::workspaces::engine`
1. Inactive tabs are rows of about 1 KB, no open document (§11.16, step 1).
2. On switch, open only each pane's visible tab; thumbnails for the rest (steps 2, 7).
3. Warm cache for the last one or two workspaces; hibernate least recently used; never hibernate unsaved work
   (steps 3, 5).
4. No file handles held by inactive tabs (step 6); debounced writes to the three tables (step 8).

*Built (2026-11-15):* steps 1, 2 (without thumbnails), 6 and 8. The warm cache and hibernation (steps 3, 5) are
not built yet: a switch closes documents no pane shows.

**Done when:** the budgets of §11.16 pass, and switching 1,000 times leaks no memory.

### 27.6 Annotation (A)

All PDF annotations are created through `excellent-core::annot` (K.14), so each one gets an `/NM` id, an author
(`pdf_set_annot_author`), dates (`pdf_set_annot_modification_date`) and an anchor (K.28), and runs inside one
operation. After changing any property, call `pdf_update_annot` to rebuild the appearance stream. While an annotation
is being edited, hide it from tiles with `pdf_set_annot_hidden_for_editing` and draw it live in the canvas (K.7,
step 5). Non-PDF formats keep the same model in the sidecar (§9.2).

#### Ph3: 14/49 | A.1: Text markup: highlight, underline, squiggly, strikeout
**P0; Phase 3; Needs:** K.13, K.14; **Code:** `excellent-features/annotate/markup.rs`
1. Selection to quads with `fz_highlight_selection` (K.13).
2. `pdf_create_annot` with `PDF_ANNOT_HIGHLIGHT`, `UNDERLINE`, `SQUIGGLY` or `STRIKE_OUT`; then
   `pdf_set_annot_quad_points`, `pdf_set_annot_color`, `pdf_set_annot_opacity`.
3. Floating selection toolbar drawn in the canvas: the four markups and a color row; shortcuts in Annotate mode.
4. Extending an existing highlight over adjacent text merges into it instead of stacking.

**Done when:** markups show correctly in Acrobat, PDF-XChange and pdf.js (K.26); they stay on the text under every
rotation.

*Built (2026-11-15):* the selection's quads come from the canvas's own text (K.13's `PageText::highlight`), already
in page space, so rotation doesn't touch them. The bar's colours are a fixed five until A.3's palettes. Merging
compares runs on their upright bounds (same line: heights overlap by half; touching: a gap under a quarter of the
line's height). The author is the system's user name: the plan names no setting for it. The page keys are `a`
(SumatraPDF's), `u`, `Shift+U` and `x`; there are no Annotate-only keys, so they work in every mode.

#### Ph3: 15/49 | A.2: Replace and insert text
**P0; Phase 3; Needs:** A.1; **Code:** as A.1
1. Insert: a `Caret` annotation at the cursor with the proposed text as contents.
2. Replace: a strikeout over the selection plus a caret, grouped with `/IRT` and `/RT /Group` (set on
   `pdf_annot_obj` with `pdf_dict_put`), as Acrobat does.

**Done when:** Acrobat shows the pair as one "replace text" comment.

*Built (2026-11-15):* the caret is made first, then the strikeout with `/IRT` naming it and `/RT /Group`, so the
shim finds the caret on the page by its `/NM`; `Annotation::group` carries the link through the log and the file.
The caret's rectangle is MuPDF's 20 × 14 pt centred on the foot of the edge the text goes in at. Insert goes after
the selection, or at the caret the last plain click in the text left. The page keys are `E` and `Insert` (`R` and
`I` are SumatraPDF's reload and page info). A replacement across pages proposes only its last page's part.

#### Ph3: 16/49 | A.3: Annotation colors
**P0; Phase 3; Needs:** K.17; **Code:** `excellent-features/annotate/colors.rs`
1. A default palette per tool from the theme's annotation colors (§10.3); recent colors; NSH `ColorPicker` for custom.
2. Opacity per tool.

**Done when:** palettes include a color-blind-safe set (U.4).

*Built (2026-11-15):* the palettes are pure data in `annotate::colors`: highlighters (highlight, squiggly) take five
light swatches, the line tools (underline, strikeout, the proposals) five strong inks; the colour-blind-safe set is
Okabe and Ito's. A test holds the standard highlighter swatches equal to the theme's `annot-*` colours. Six custom
colours are remembered, newest first. The canvas gets every tool's colour and opacity as its `annotationColors` prop
and tells the app a swatch's choice (`toolColor`) or asks for a custom colour (`pickColor`). Opacity is the picker's
alpha, at least 10 %. The command is `annotate.colors` (`Shift+C`); it opens on the highlighter.

#### Ph3: 17/49 | A.4: Notes on highlights
**P0; Phase 3; Needs:** A.1; **Code:** `excellent-features/annotate/notes.rs`
1. Contents with `pdf_set_annot_contents`; a popup with `pdf_set_annot_popup` so other readers can open it.
2. Edit in a popover beside the page (not over it, §7.3) and in the inspector; Markdown-light text stored as plain text.

**Done when:** notes open in other readers' comment panes.

*Built (2026-11-15):* the shim adds the `/Popup` (200 by 120 points, right of the markup, kept on the page) when a
markup with contents is written and has none. The canvas sends a plain click on the page as `pagePress`; the app
finds the markup under it (`notes::note_at`, top one first) and opens its note in the inspector, which the window
reveals. The markup bar's Note tool (`annotate.note`, `Shift+N`) highlights and opens the new highlight's note
(`notes::marked`). The note is saved on Done, or when another opens, as one undoable change (`notes::write`). The
editor is the inspector only; the separate popover waits for A.12's inspector.

#### Ph3: 18/49 | A.5: Sticky notes
**P0; Phase 3; Needs:** K.14; **Code:** as A.4
1. `PDF_ANNOT_TEXT` at the click point, `pdf_set_annot_icon_name` (Comment, Note, Help, Insert, Key, Paragraph,
   NewParagraph), `pdf_set_annot_is_open`.

**Done when:** icons and open state match in Acrobat.

*Built (2026-11-16):* `Annotation` has `icon` (the file's own `/Name`, empty for none, so a read gives back what was
written; MuPDF's default name is not) and `open` (`pdf_annot_is_open`: the note's `/Open`, or its popup's once it has
one). The command `annotate.sticky` (`Shift+S`) waits for the next `pagePress` and places the note with its top left
at the click (`sticky::place`, 20 points, kept inside the page bounds). The note editor is A.4's, with the icon and
the open switch added for sticky notes.

#### Ph3: 19/49 | A.6: Text boxes and typewriter
**P0; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/freetext.rs`
1. `PDF_ANNOT_FREE_TEXT`; font, size and color with `pdf_set_annot_default_appearance`; alignment; border.
2. Typewriter: `pdf_set_annot_intent(PDF_ANNOT_IT_FREETEXT_TYPEWRITER)`, no border, fills forms that have no fields.
3. Editing text in place uses the platform input path (U.8) so input methods work.

**Done when:** text boxes typed in Chinese with an input method save and show in Acrobat.

*Built (2026-11-16):* `Shape::Boxed(Boxed::FreeText | Boxed::Typewriter, _)` with `Annotation::text`
(`TextStyle`: font, size, colour, alignment); the shim writes `/DA` only when the style changed, since MuPDF's setter
drops `/RC` and `/DS`. A text box's `Style::color` is its background and `Style::width` its border; MuPDF strokes the
border in the text colour. Boxes are fitted to their text by `freetext` (line height 1.2 em, MuPDF's) from average
character widths, CJK a full em. Until U.8, the text is typed in the inspector's NSH input, which already takes input
methods; step 3 (in place on the page) waits for U.8.

#### Ph3: 20/49 | A.7: Callouts with leader lines
**P0; Phase 3; Needs:** A.6; **Code:** as A.6
1. `PDF_ANNOT_IT_FREETEXT_CALLOUT`; `pdf_set_annot_callout_line`, `pdf_set_annot_callout_point`,
   `pdf_set_annot_callout_style`.
2. Handles for the text box and the arrow tip.

**Done when:** moving the box keeps the tip in place.

*Built (2026-11-16):* `Shape::Callout(rect, tip)`; the shim writes `/CL` from the tip alone with
`pdf_set_annot_callout_point` (MuPDF computes the point on the box's edge) and the tip's ending with
`pdf_set_annot_callout_style`; `pdf_set_annot_callout_line` isn't needed. `LineEnd` is the model's `/LE` for every
kind, so A.8 reuses it. The handles (step 2) wait for A.13, which gives every annotation its selection handles.

#### Ph3: 21/49 | A.8: Lines and arrows
**P0; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/shapes.rs`
1. `PDF_ANNOT_LINE`; `pdf_set_annot_line`; ends with `pdf_set_annot_line_ending_styles` (open, closed, circle,
   square, diamond, slash).
2. Shift snaps to 15° steps.

**Done when:** arrow heads look the same in Acrobat.

*Built (2026-11-16):* `Shape::Line` with `Annotation::ends` (A.7's `LineEnd`); the shim writes them with
`pdf_set_annot_line_ending_styles` for lines, polylines and polygons. The canvas draws the line as it is dragged (the
`drawing` prop, the `pageDraw` event) and asks its host whether Shift is down (`Host::shift_down`; Windows reads it,
GTK and macOS answer no until their hosts do). Shapes take their colour from a new colour tool, `Shape` (A.3).

#### Ph3: 22/49 | A.9: Rectangles and ellipses
**P0; Phase 3; Needs:** K.14; **Code:** as A.8
1. `PDF_ANNOT_SQUARE` and `PDF_ANNOT_CIRCLE`; `pdf_set_annot_rect`, `pdf_set_annot_interior_color`,
   `pdf_set_annot_border_width`, dashes with `pdf_add_annot_border_dash_item`.
2. Shift draws squares and circles.

**Done when:** fill, border and dash survive a round trip.

*Built (2026-11-16):* `Annotation::fill` (`/IC`) and `Annotation::dash` (`/BS /D`, empty for solid); the shim reads
and writes them with `pdf_set_annot_interior_color`, `pdf_clear_annot_border_dash`, `pdf_add_annot_border_dash_item`
and `pdf_set_annot_border_style`, and leaves a border it didn't change as it was. A.8's drawing tool draws the boxes
(`shapes::Kind::Rectangle` and `Ellipse`); Shift goes through `shapes::constrain`. Width and line style are offered
for lines too.

#### Ph3: 23/49 | A.10: Polygons, polylines and clouds
**P0; Phase 3; Needs:** A.8; **Code:** as A.8
1. `PDF_ANNOT_POLYGON` and `PDF_ANNOT_POLY_LINE` with `pdf_set_annot_vertices` (or `pdf_add_annot_vertex` while
   drawing); double-click finishes.
2. Clouds: `pdf_set_annot_border_effect(PDF_BORDER_EFFECT_CLOUDY)` and `pdf_set_annot_border_effect_intensity`.
3. Polygon and star tools with side and point counts, like Enhanced's `PolygonSides` and `StarSides`.

**Done when:** clouds render the same in Acrobat.

*Built (2026-11-16):* `Annotation::cloud` (`/BE`, `None` for a plain border) through the shim's `cloudy` and
`cloud_intensity`, written with `pdf_set_annot_border_effect` and `pdf_set_annot_border_effect_intensity` only when
they change. The geometry is `annotate::polygons` (`regular`, `star`, `distance`, `inside`); `shapes::Kind` gains
`Regular(n)`, `Star(n)`, `Polygon`, `PolyLine` and `Cloud`, named `regular:5` and so on in the canvas's `drawing`
prop. The canvas counts clicks with K.13's click counter for the double-click; the vertices go to the app in one
`pageDraw`, so the whole shape is one step of undo (no `pdf_add_annot_vertex` while drawing). Settings
`polygon_sides` and `star_points`.

#### Ph3: 24/49 | A.11: Margin notes and anchored note cards
**P0; Phase 3; Needs:** A.4, K.28; **Code:** `excellent-features/annotate/margin.rs`
1. A margin column beside the page (canvas overlay) showing each annotation's note as a card, aligned with its
   anchor and pushed apart when they overlap.
2. Cards are edited in place; clicking a card selects its annotation.
3. "Expand margins" (N.3) gives more room on PDFs.

**Done when:** 200 notes on one page lay out without overlap within one frame.

*Built (2026-11-16):* the layout is `annotate::margin::layout`: cards sorted by anchor and merged into clusters
centred on their anchors, linear after the sort. The canvas reports each page in view as a map from page points to
the app's pixels (`margin` events, prop `margin`; `Host::ui_pixels_per_point` for the platform's UI unit); the column
is an NSH view beside the canvas (DD11), its cards read again only when other pages come into view or the document is
edited. Setting `margin_notes`. Item 3 waits for N.3.

#### Ph3: 25/49 | A.12: Annotation inspector
**P0; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/inspector.rs`
1. Right panel: type, author, dates, color, opacity, border, font, contents, category (A.17), tags (A.18), layer
   (A.22), lock.
2. Multi-selection edits shared properties.

**Done when:** every property set here is visible in the next frame and undoable.

*Built (2026-11-16):* `annotate::inspector`: `Details`, `set_author`, `set_color`, `set_opacity`, `set_locked`
(each one change), `allows` (a locked annotation takes only its note and its lock) and `date`. The lock is PDF's
`/F` Locked bit (`ex_annot_props.locked`, in the struct's old padding). The app's rows sit under the note and the
kind's controls, built again from the annotation after every revision. Border and font were A.6's and A.9's rows
already. Category, tags and layer wait for A.17, A.18 and A.22; item 2 waits for A.13's selection.

#### Ph3: 26/49 | A.13: Select, move, resize, copy, paste and delete
**P0; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/edit.rs`
1. Hit-test annotations in the canvas; handles for move, resize and vertices; marquee and Ctrl-click multi-select.
2. Copy and paste within and across documents (internal clipboard format plus an image and text fallback).
3. Arrow keys nudge; Delete removes; lock prevents changes (`pdf_set_annot_flags` with the locked flag).

**Done when:** pasting into another document keeps type, style and contents.

*Built (2026-11-16):* `annotate::edit`: `grips`, `dragged`, `moved`, `change` (honours A.12's lock), `delete`,
`in_marquee`, `toggled`, `hit`, the canvas's `Drag` and `Marquee`, and `Clip` (JSON format
`excellentpdf.annotations/1`; a paste gets new ids and dates, unlocked, offset on its own page). The canvas
(`page/canvas/select.rs`) draws the app's `selected` prop and reports `annotDrag` and `annotMarquee`; `pagePress`
carries `add` (Ctrl). The app's `annotate/select.rs` keeps the selection and makes each edit one operation. The
clipboard is the app's own, with the notes as text on the system clipboard; the image fallback waits (DD12).

#### Ph3: 27/49 | A.14: Annotation list with filter and search
**P0; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/list.rs`
1. Sidebar tab with NSH `virtual_list`: one row per annotation (type icon, color, text, page), grouped by page.
2. Filter by type and color; search in contents and marked text (as Enhanced's `src/AnnotSearch.cpp` and
   `src/AnnotFilterToolbar.cpp`).
3. Click jumps and selects.

**Done when:** 10,000 annotations filter as you type within 16 ms.

*Built (2026-11-16):* `annotate::list`: `Kind`, `Row` (text folded to lower case once), `Filter::apply`, `offered`
and `entries` (page headings and rows). `excellent-core::annot::load_all` reads every page's annotations at
background priority, cancellable. The app's `annotate/list.rs` is the panel, a tab of the right dock beside the
inspector (DD13), with `annotate.list` (Shift+O on the page) to show it.

#### Ph3: 28/49 | A.15: Annotation recovery
**P0 (parity); Phase 3; Needs:** K.15; **Code:** `excellent-core::save`
1. Enhanced's recovery (`src/AnnotRecovery.cpp`) is replaced by K.15's log replay; add a "Recovered changes" bar that
   lists what was restored and lets the user keep or discard it.

**Done when:** killing the app after drawing ten annotations restores all ten on restart.

*Built (2026-11-16):* `Document::recover` returns the names of the edits it brought back (`oplog::Read::restored`),
and `Document::revert` drops every unsaved edit and starts the log again. The app (`editing.rs`) recovers when the
file is shown, without asking, and saves nothing until the reader decides; `recovery_bar.rs` is the bar.

#### Ph3: 29/49 | A.16: Transfer annotations to another version
**P1 (parity); Phase 3; Needs:** K.28; **Code:** `excellent-features/annotate/transfer.rs`
1. Pick a source and target document; re-anchor each annotation by text (K.28); show a review list (placed, moved,
   orphaned) before applying.
2. Port the ideas of Enhanced's `src/AnnotationTransfer.cpp`.

**Done when:** transferring from a preprint to the published version places 95 % of highlights correctly.

*Built (2026-11-16):* `annotate::transfer`: text markups are quoted (their K.28 anchor, else the text under their
quads) and located in the target's flat text; `Fate` is placed, moved, in place (no text to find) or orphaned.
`excellent-core::annot::load_text` reads every page's text on the workers. The app's `annotate/transfer.rs` picks
the source, reviews in a dialog (DD14) and adds the placed ones as one operation.

#### Ph3: 30/49 | A.17: Semantic categories
**P1; Phase 3; Needs:** A.12; **Code:** `excellent-features/annotate/categories.rs`
1. Categories (claim, evidence, question, definition, method, result, to-do) each with a color and icon; stored in a
   private key on the annotation and in the sidecar.
2. Pick from the selection toolbar; filter and export by category.

**Done when:** categories survive save and reopen and show in A.26 reports.

*Built (2026-11-16):* `excellent-core::annot::Annotation::category` keeps the key in the private `/ExcellentPDF`
JSON (and the operation log); `annotate::categories` has the seven, their colours and Lucide icon names, and
`set_category`. The picker is in the inspector (DD15); `annotate::list` filters by category. Export and the A.26
reports come with A.26.

#### Ph3: 31/49 | A.18: Tags on annotations
**P1; Phase 3; Needs:** A.12, K.20; **Code:** as A.17
1. Free-form tags stored on the annotation (private key) and indexed in `library.db` for library-wide search (F.3).

**Done when:** `tag:` search (F.4) finds tagged annotations across documents.

*Built (2026-11-16):* `annotate::tags` (`set_tags`, `asked` for `tag:` words); the inspector's Tags field; the
annotation list's search takes `tag:`. `library.db` migration 6 adds `annotation_tag`, written after every save
(`excellent-app::annotate::tag_index`); `excellent_store::annotation_tags::find` is the query F.4's `tag:` will run
(DD16).

#### Ph3: 32/49 | A.19: Filters by color, tag, author and date
**P1; Phase 3; Needs:** A.14, A.18; **Code:** as A.14
1. Filter chips in the list; "show only matching" also hides the others on the page (render with annotations
   filtered).

**Done when:** filters combine with AND and are saved per document.

*Built (2026-11-16):* `annotate::list::Filter` gains tags, authors, `Since` and `show_only`, with `hidden` and JSON;
`library.db` migration 7 adds `annotation_filter`. Pages are recorded without hidden annotations by the shim's
`ex_new_display_list_without` (`Page::display_list_without`, `Document::hide_annotations`). Printing and export
keep every annotation (DD17).

#### Ph3: 33/49 | A.20: Stamps library with dynamic fields
**P1; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/stamps.rs`
1. Standard stamps: `PDF_ANNOT_STAMP` with `pdf_set_annot_icon_name` (Approved, Draft, Confidential…).
2. Image stamps: `pdf_set_annot_stamp_image`; custom vector stamps: appearance from a display list
   (`pdf_set_annot_appearance_from_display_list`).
3. Dynamic fields (date, time, user name, page) filled when placed.
4. A library of user stamps saved as small PDF files; import from PDF or image.

**Done when:** stamps look the same in Acrobat; dynamic fields show the placement time.

*Built (2026-11-16):* `annotate::stamps` (standard, dynamic, library); `Change::StampArt` sets a stamp's art after its
`CreateAnnotation` in the same operation, through the shim's `ex_annot_set_stamp_art` (a one-page PDF's display list
as the appearance, or `pdf_set_annot_stamp_image`). Dynamic stamps are HTML laid out by MuPDF's story engine
(`html_to_pdf`); a PDF imported to the library is its first page written by `Page::to_pdf`. Images stay images in
the library and times are UTC (DD18).

#### Ph3: 34/49 | A.21: Comment threads with replies and resolve
**P1; Phase 3; Needs:** A.4; **Code:** `excellent-features/annotate/threads.rs`
1. A reply is a `Text` annotation with `/IRT` pointing to its parent and no visible icon.
2. Status (accepted, rejected, cancelled, completed) as a reply with `/State` and `/StateModel /Review`.
3. Thread view in the inspector and in the list (A.14).

**Done when:** threads and states show in Acrobat's comment pane, and Acrobat's replies show here.

*Built (2026-11-16):* `annotate::threads` (reply, set_status, thread, status_of); `Annotation::reply_to` and
`state` (`Status`), written by the shim's `ex_annot_reply` (`/IRT` without `/RT`; `/State`, `/StateModel`, `/F 30`).
Display lists and `ex_render_page` leave replies out, `page_annotations` too (`page_replies` reads them), and
`ex_annot_delete` deletes a thread with its parent (DD19). The list counts replies and shows the status.

#### Ph3: 35/49 | A.22: Annotation layers by person or purpose
**P1; Phase 3; Needs:** A.12; **Code:** `excellent-features/annotate/layers.rs`
1. Create optional content groups in `/OCProperties` (MuPDF has no helper, so write the dictionaries with
   `pdf_dict_put`) and set each annotation's `/OC`.
2. Layer panel: show, hide, lock; MuPDF reads visibility through `pdf_enable_layer` and `pdf_layer_is_enabled`.
3. Default layers per author.

**Done when:** hiding a layer here hides it in Acrobat's layers panel too.

*Built (2026-11-16):* `annotate::layers` (move_to, by_author, show, lock, shown, place_new); `Annotation::layer`
written as `/OC` by the shim's `ex_annot_layer`, which makes the group in `/OCGs` and `/D /Order`; `Change::SetLayer`
writes `/D`'s `/OFF`, `/ON` and `/Locked` through `ex_layer_set`, and MuPDF's layer cache is reset after each change,
undo and redo. Showing or hiding redraws every page (`Pages::Every`). The Layers section heads the annotation list;
the inspector has a Layer row (DD20).

#### Ph7: 4/32 | A.23: Measurement
**P1; Phase 7; Needs:** A.8, A.10; **Code:** `excellent-features/measure`
1. Tools: distance (`PDF_ANNOT_IT_LINE_DIMENSION`), perimeter (`PDF_ANNOT_IT_POLYLINE_DIMENSION`), area
   (`PDF_ANNOT_IT_POLYGON_DIMENSION`), angle.
2. Scale: read the page's viewport `/VP` and `/Measure` dictionaries when present; otherwise calibrate by drawing a
   known length. Write `/Measure` on each annotation so other readers can show the value.
3. Units: metric and imperial; precision setting; the value as the line caption (`pdf_set_annot_line_caption`).
4. Snap to vector line ends and intersections (from `FZ_STEXT_COLLECT_VECTORS` paths).

**Done when:** measurements match Acrobat's on calibrated CAD files within 0.5 %.

#### Ph3: 36/49 | A.24: XFDF and FDF import and export
**P1; Phase 3; Needs:** K.14; **Code:** `excellent-features/annotate/xfdf.rs`
1. Write and parse XFDF in Rust (`quick-xml`); consider reusing PdfCraft's XFDF code (Appendix C.5).
2. FDF: MuPDF can open FDF files (`is_fdf`); map their `/Annots` into the document.
3. Import merges by `/NM`: same id updates, new id adds.

**Done when:** XFDF exported here imports into Acrobat and back without loss.
**Links:** XFDF is ISO 19444-1; quick-xml <https://github.com/tafia/quick-xml>

*Built (2026-11-16):* `annotate::xfdf` (`write`, `read`, `merge`, `PageSpace`) over `AnnotData`, written fresh with
`quick-xml` 0.42 (0.41 fixes RUSTSEC-2026-0194 and 0195) rather than from PdfCraft; FDF through the shim (`ex_fdf_write` grafts the annotations into an FDF
body with `/Page`; `ex_fdf_read` grafts an FDF's onto a blank copy of the pages, read as the document's own are), and
`ex_page_transform` for PDF space. The app's `annotate::exchange` adds Export (Ctrl+Alt+E) and Import (Ctrl+Alt+I),
one undo step per import (DD21).

#### Ph3: 37/49 | A.25: Flatten on export
**P1; Phase 3; Needs:** K.15; **Code:** `excellent-features/annotate/flatten.rs`
1. "Export flattened copy": save a copy, then `pdf_bake_document(ctx, doc, bake_annots = 1, bake_widgets = 0 or 1)`;
   never flatten the open original.

**Done when:** a flattened copy shows the same pixels and has no annotations left.

*Built (2026-11-16):* the shim's `ex_save_flattened` writes the document (edits included) to memory, reopens it and
bakes that copy, so the open original is never touched; `Document::save_flattened(path, Forms)`. No
`excellent-features` module: there is no model beyond the engine call, so the command lives in the app's
`annotate::flatten` (Ctrl+Shift+E). Form fields stay fillable (DD22).

#### Ph5: 13/45 | A.26: Annotation reports
**P2; Phase 5; Needs:** A.14; **Code:** `excellent-features/annotate/report.rs`
1. Export all or filtered annotations as Markdown, HTML, CSV or PDF summary, with page, quote, note, category and a
   small page crop.

**Done when:** the Markdown report links back to each page (F.13 deep links).

#### Ph5: 14/45 | A.27: Annotation history timeline
**P2; Phase 5; Needs:** K.14; **Code:** `excellent-features/annotate/history.rs`
1. Read the operation log (K.14): a timeline of who changed what and when; scrub to see the document at a time.
2. Restore one annotation, or the whole document, to an earlier state as a new operation.

**Done when:** restoring an annotation deleted a week ago works.

#### Ph3: 38/49 | A.28: W3C Web Annotation export
**P1; Phase 3; Needs:** K.28; **Code:** `excellent-features/annotate/w3c.rs`
1. Export and import annotations as W3C Web Annotation JSON-LD with text-quote and text-position selectors.

**Done when:** exported JSON validates against the model's JSON-LD context.
**Links:** <https://www.w3.org/TR/annotation-model/>, <https://www.w3.org/TR/annotation-vocab/>

*Built (2026-11-16):* a third format of A.24's Export and Import commands, not commands of its own. Each annotation
also carries its whole record under the IRI `urn:excellentpdf:annotation`, so our own files round-trip; other tools'
quotes are placed by A.16's transfer. The test checks every term against the context's list (DD23).

### 27.7 Handwriting, ink and drawing (H)

The pipeline is in §8. Ink lives in `crates/excellent-ink` (model, smoothing, outlines) and the platform canvases
(K.8–K.12, capture and same-frame preview).

#### Ph0: 13/13 | H.1: Ink capture and stroke pipeline
**P0; Phase 0 spike, Phase 3 build; Needs:** K.8; **Code:** `excellent-ink`, canvas crates; task T8
1. Capture every sample, coalesced ones included: position, pressure, tilt, azimuth, timestamp, tool (§8.2, step 1).
2. Same-frame preview on the canvas's own surface (step 2); prediction (step 3); Catmull-Rom smoothing to Béziers
   (step 4).
3. Width from pressure, tilt and speed with per-pen response curves (step 5); commit on pen-up as one operation.
4. Record pen sessions as test fixtures and replay them in CI (§16).

**Done when:** pen-down to pixels is within one display frame on the reference Windows device (measured, §13).
**Reference:** Enhanced's pen path: `src/PointerInput.h` (sample format), `src/Canvas.cpp` (`WM_POINTER` handling and
pen logging), `src/AnnotPlacement.cpp` (`AddInkPressure`: Enhanced keeps the stroke's mean pressure because a PDF ink
annotation has one width; we keep per-point widths, §8.3).
**Links:** Windows pointer input <https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerupdate>

*Built (2026-11-16):* `excellent-ink`'s `sample`, `width`, `predict`, `smooth`, `stroke` and `session` modules; the
canvas sends a finished stroke as an `ink` event and the app commits it (`excellent-features::annotate::ink`). The
annotation takes the stroke's mean width until H.9; the session fixture is synthetic until one is recorded (DD24).

#### Ph3: 39/49 | H.2: Pen types
**P0; Phase 3; Needs:** H.1; **Code:** `excellent-ink::pens`
1. Fountain pen, ballpoint, pencil (grain texture, tilt shading), brush, calligraphy (nib angle), marker; each a
   preset of width curve, smoothing, texture and blend.
2. Port nib behaviour from Enhanced's pen tools where it exists (`src/Canvas.cpp` and the `Canvas_UnitTest*` tests).

**Done when:** each pen has a recorded-session golden image test.

*Built (2026-11-16):* `excellent-ink::pens` (the six presets and the calligraphy nib) and `excellent-ink::raster`
(the golden-image rasterizer); `ink.pen_*` commands on the Write toolbar (keys 1 to 6) set the canvas's `pen` prop.
Enhanced has no pen-type code to port. Texture and blend reach the saved appearance with H.9; the pen is per pane
until H.7 (DD25).

#### Ph3: 40/49 | H.3: Highlighter pen
**P0; Phase 3; Needs:** H.2; **Code:** as H.2
1. Wide translucent stroke with blend mode Multiply in the appearance stream; optional snap to text lines (K.13).

**Done when:** text under the highlighter stays fully readable in light and dark mode.

*Built (2026-11-25):* `PenKind::Highlighter` (a 12 pt chisel, `Blend::Multiply`, yellow) on the Write toolbar (key 7);
the shim writes and reads `/BM /Multiply` on the Ink annotation, and MuPDF patch `0094` makes its ink appearance
honour it. The live preview skips the system trail (it draws opaque) and draws segment quads darkest-wins. Snapping
to text lines is not built yet.

#### Ph4: 3/44 | H.4: Tape
**P1; Phase 4; Needs:** H.2; **Code:** `excellent-ink::tape`
1. An opaque strip that hides what is under it; tap to reveal or hide (for self-testing); saved as a Square
   annotation with a private "tape" key, so other readers show a plain bar.

**Done when:** tape toggles with one tap and survives save.

*Built (2026-11-25):* `excellent-features::annotate::tape` (a filled Square with the private `tape` mark; a tap is one
undoable opacity change between covering and revealed) and the Tape tool on the Write toolbar. The model sits with
the annotation models: it has no ink geometry for `excellent-ink`.

#### Ph3: 41/49 | H.5: Erasers
**P0; Phase 3; Needs:** H.1; **Code:** `excellent-ink::erase`
1. Stroke eraser (removes whole strokes it touches), area eraser (splits strokes at the eraser outline),
   highlighter-only eraser.
2. The stylus's eraser end selects the eraser automatically (pointer flags on each platform).

**Done when:** area-erasing through a stroke leaves two correct strokes, both undoable in one step.

*Built (2026-11-25):* `excellent-ink::erase` (capsule geometry: whole strokes, exact area cuts, highlighter ink only),
`excellent-features::annotate::erase` (one operation of updates and deletions; locked and hidden-layer ink kept), the
canvas's `eraser` prop and eraser end with an `erase` event on pen-up, and `ink.eraser_*` on the Write toolbar (keys
8, 9, 0). Ink goes on pen-up rather than live; an area eraser's pieces stay in their one annotation.

#### Ph3: 42/49 | H.6: Lasso
**P0; Phase 3; Needs:** H.5, A.13; **Code:** `excellent-ink::lasso`
1. Freeform or rectangle lasso (Enhanced's `LassoRectangular`) selects strokes and annotations inside the path.
2. Move, resize, rotate, duplicate, recolor, change width, delete, copy as image, convert to text (H.24).
3. Port Enhanced's lasso geometry tests.

**Done when:** a lasso move of 1,000 strokes stays at 60 fps.

*Built (2026-11-25):* `excellent-ink::lasso` (Enhanced's even-odd loop and turning, ported; Enhanced has no lasso
tests, so the tests are new), `excellent-features::annotate::lasso` (ink taken when wholly inside, others by their
corners; group resize from the selection's own box, quarter turns, widths × 0.8 or × 1.25, recolour, duplicate),
the canvas's `lasso` prop and `annotLasso`, and the inspector's selection panel. The rectangle lasso is A.13's
marquee. Not built: a free-turning handle, copy as image (O.23), convert to text (H.24).

#### Ph3: 43/49 | H.7: Favourite pens bar and presets
**P0; Phase 3; Needs:** H.2; **Code:** `excellent-features/ink/favorites.rs`
1. A bar of saved pens (type, color, width, curve); drag to reorder; export and import as JSON.

**Done when:** the bar syncs through settings (Y.2).

*Built (2026-11-25):* `excellent-features::ink::favorites` (up to 12 pens: kind, colour, width, pressure response;
settings JSON and export files), the bar at the start of the Write toolbar (press, drag to reorder, right-click to edit
or remove, `+` to save the pen in hand), its edit dialog, and Export / Import in the palette. It lives in
`settings.toml` under `[ink]`, so it syncs with the settings (Y.2, Phase 9). This ends DD25's per-pane pen.

#### Ph3: 44/49 | H.8: Palm rejection and "finger draws"
**P0; Phase 3; Needs:** H.1; **Code:** `excellent-canvas::gestures`
1. While a pen is in range or down, ignore touches; ignore large contact areas.
2. "Finger draws" toggle; otherwise fingers pan and zoom.

**Done when:** writing with the hand resting on a touch screen leaves no stray marks in a recorded test.

*Built (2026-11-25):* `excellent-canvas::page::gestures` (touches ignored while the pen is near or on the screen
and 500 ms after, or wider than 16 mm; a verdict kept until the touch lifts), fed by the Windows canvas
(`WM_POINTER*` pen hover and leave, `GetPointerTouchInfo` contact width); "Finger draws" in Settings, Modes. The
recorded test is a synthetic session; macOS and Linux hosts are not wired yet.

#### Ph3: 45/49 | H.9: Save ink as standard PDF
**P0; Phase 3; Needs:** H.1, K.14; **Code:** `excellent-ink::pdf`
1. InkList with `pdf_add_annot_ink_list` (smoothed points) (§8.3).
2. Constant-width pens: `pdf_update_annot`. Pressure pens: the outline as a filled Bézier path in a display list,
   attached with `pdf_set_annot_appearance_from_display_list`.
3. Raw samples (pressure, tilt, time) in a private key, compressed, for replay (H.22) and audio sync (L.1).

**Done when:** ink looks identical in Acrobat, PDF-XChange and pdf.js (K.26).

*Built (2026-11-25):* step 1 as before; step 2 with `excellent-ink::outline` and the shim (constant-width pens keep
MuPDF's appearance; pressure pens' outlines are filled through a display list, with `/ExcellentWidths` holding each
point's width so every later write draws them again). Step 3 (raw samples) is not built. The three readers are not
checked yet.

#### Ph3: 46/49 | H.10: Shape recognition (hold to snap)
**P1; Phase 3; Needs:** H.1; **Code:** `excellent-ink::shapes`
1. When the pen stays still for 500 ms at the end of a stroke, fit: line, arrow, rectangle, triangle, polygon (corner
   detection with Douglas–Peucker), circle and ellipse (least squares).
2. Replace the stroke with the fitted shape if the error is under a threshold; keep pen style.

**Done when:** recognition accuracy is at least 95 % on a recorded set of 500 shapes.

*Built (2026-11-25):* `excellent-ink::shapes` (line, arrow, triangle, rectangle, polygon up to eight corners, circle,
ellipse; scribbles refused) and the canvas's hold (500 ms still before the pen lifts). 99.4 % on a seeded set of 500
synthetic hand-drawn shapes; a recorded set is still to come.
**Links:** $-family recognizers <https://depts.washington.edu/acelab/proj/dollar/index.html>

#### Ph4: 4/44 | H.11: Ruler, protractor and compass
**P1; Phase 4; Needs:** H.1; **Code:** `excellent-ink::guides`
1. On-canvas guides moved with two fingers or the mouse; strokes near the ruler edge snap to it; angle and length
   readouts.

**Done when:** a stroke along the ruler is exactly straight.

#### Ph4: 5/44 | H.12: Zoom writing pad
**P1; Phase 4; Needs:** H.1; **Code:** `excellent-features/ink/zoompad.rs`
1. A magnified strip at the bottom; writing in it lands small in a moving box on the page; auto-advance at the end
   of a line.

**Done when:** writing a paragraph in the pad needs no manual box moves.

#### Ph4: 6/44 | H.13: Handwriting straightening
**P1; Phase 4; Needs:** H.6; **Code:** `excellent-ink::straighten`
1. For a selection of handwriting: find each line's baseline (regression on stroke bottoms) and rotate and shift
   strokes to level, keeping their shapes.

**Done when:** a sloped paragraph becomes level without changing letter shapes.

#### Ph4: 7/44 | H.14: Layers
**P1; Phase 4; Needs:** A.22; **Code:** `excellent-features/ink/layers.rs`
1. Fixed kinds (ink, highlights, text, images) plus user layers; show, hide, lock; implemented with A.22's optional
   content groups.

**Done when:** locked layers can't be selected or erased.

#### Ph4: 8/44 | H.15: Bulk restyle
**P1; Phase 4; Needs:** H.6; **Code:** as H.6
1. Change color, width or pen type for all selected strokes, or all strokes matching a filter (for example "all red
   ink on this page").

**Done when:** one undo reverts the whole restyle.

#### Ph4: 9/44 | H.16: Fill and gradients
**P1; Phase 4; Needs:** H.6; **Code:** `excellent-ink::fill`
1. Fill a closed stroke or shape with a solid color or a linear or radial gradient, written in the appearance stream.

**Done when:** fills show the same in Acrobat.

#### Ph4: 10/44 | H.17: Eyedropper
**P1; Phase 4; Needs:** H.2; **Code:** `excellent-features/ink/eyedropper.rs`
1. Pick a color from the rendered page or any annotation.

**Done when:** the picked color is exact (from the pixmap, before dark mode).

#### Ph4: 11/44 | H.18: Saved palettes
**P1; Phase 4; Needs:** A.3; **Code:** `excellent-features/ink/palettes.rs`
1. Built-in palettes (pastel, neon, earth, color-blind-safe) and user palettes, shared as small files.

**Done when:** palettes import and export.

#### Ph5: 15/45 | H.19: Handwriting beautify
**P2; Phase 5; Needs:** H.13; **Code:** `excellent-ink::beautify`
1. Smooth jitter, even out letter size and spacing within a line, with a strength slider; reversible.

**Done when:** text stays recognizable to H.24 after beautifying.

#### Ph5: 16/45 | H.20: Pattern brushes
**P2; Phase 5; Needs:** H.2; **Code:** `excellent-ink::brushes`
1. Brushes that repeat a small vector pattern along the path; appearance as repeated paths.

**Done when:** pattern strokes render the same in other readers.

#### Ph5: 17/45 | H.21: Symmetry
**P2; Phase 5; Needs:** H.1; **Code:** `excellent-ink::symmetry`
1. Mirror strokes across a vertical, horizontal or radial axis while drawing.

**Done when:** mirrored strokes are saved as separate ink.

#### Ph4: 12/44 | H.22: Stroke replay
**P2; Phase 4; Needs:** H.9, K.14; **Code:** `excellent-features/ink/replay.rs`
1. Replay a page's ink in the order and timing it was written (from the raw samples), with play, pause and speed.

**Done when:** replay timing matches the recording.

#### Ph5: 18/45 | H.23: Searchable handwriting index
**P0; Phase 5; Needs:** H.9, F.3; **Code:** `excellent-features/handwriting/index.rs`
1. A `Recognizer` trait with back ends: Windows ink analysis (`Windows.UI.Input.Inking.Analysis.InkAnalyzer` through
   the `windows` crate), Apple Vision text recognition on rendered strokes (macOS, iPad), ML Kit Digital Ink on
   Android, and an ONNX model pack for Linux and as a fallback. Choose and measure in a spike.
2. Recognize new strokes at idle priority; store words with stroke ids and boxes; index them in FTS5 (F.3).
3. Search hits highlight the strokes.

**Done when:** searching a handwritten word in a notebook finds it on every platform with a recognizer.
**Links:** InkAnalyzer <https://learn.microsoft.com/en-us/uwp/api/windows.ui.input.inking.analysis.inkanalyzer>;
ML Kit Digital Ink <https://developers.google.com/ml-kit/vision/digital-ink-recognition>; Apple Vision
<https://developer.apple.com/documentation/vision>

#### Ph5: 19/45 | H.24: Convert handwriting to text
**P1; Phase 5; Needs:** H.23, H.6; **Code:** `excellent-features/handwriting/convert.rs`
1. Lasso → recognize → show candidates → replace with a FreeText annotation (A.6) or copy as text.

**Done when:** conversion keeps position and line breaks.

#### Ph6: 2/13 | H.25: Math to LaTeX
**P1; Phase 6; Needs:** H.6, K.21; **Code:** `excellent-features/handwriting/math.rs`
1. Lasso → render to image → an image-to-LaTeX ONNX model pack (for example pix2tex) → LaTeX shown and rendered for
   confirmation; insert as text, or copy.

**Done when:** at least 80 % of a test set of handwritten formulas convert without edits.
**Links:** LaTeX-OCR (pix2tex) <https://github.com/lukas-blecher/LaTeX-OCR>; ort (ONNX Runtime for Rust)
<https://github.com/pykeio/ort>

#### Ph4: 13/44 | H.26: Convert shapes to vectors
**P1; Phase 4; Needs:** H.10; **Code:** as H.10
1. Lasso → convert recognized strokes into Line, Square, Circle and Polygon annotations (A.8–A.10).

**Done when:** converted shapes are editable with A.13 handles.

### 27.8 Notebooks and notes (N)

#### Ph4: 14/44 | N.1: Blank notebooks with procedural paper
**P0; Phase 4; Needs:** N.4, K.8; **Code:** `excellent-features/notebook/paper.rs`
1. Templates as small parameter sets (blank, lined, college-ruled, grid, dot, Cornell, music, graph, isometric,
   planner): spacing, margins, colors.
2. The canvas draws the template under the page (not in tiles), so it costs no file size.
3. Exporting to plain PDF (N.4) writes the template as vector content, so other readers show it.
4. Page sizes: A4, Letter, A5, phone, custom; paper color from the theme (R.15).

**Done when:** templates look identical on screen and in an exported PDF.

#### Ph4: 15/44 | N.2: Insert pages anywhere
**P0; Phase 4; Needs:** K.14; **Code:** `excellent-features/notebook/pages.rs`
1. "Insert page before / after" in PDFs and notebooks: blank, template (N.1), or copy of the current page size.
2. PDF: `pdf_add_page` then `pdf_insert_page`; the page's template id goes in the sidecar. The full page organizer
   (O.1) comes later and reuses this code.

**Done when:** inserted pages show in other readers, and the outline still points to the right pages (§11.14, step 5).

#### Ph4: 16/44 | N.3: Expand margins
**P0; Phase 4; Needs:** K.14; **Code:** as N.2
1. Grow the page's MediaBox and CropBox on one or more sides with `pdf_set_page_box`, so annotations can sit beside
   the content; the content stays where it is.
2. "Remove added margins" restores the original boxes (stored in a private key).

**Done when:** notes written in the margin show in Acrobat; removing margins restores the original boxes exactly.

#### Ph4: 17/44 | N.4: Notebook format (`.expdf`) and export to PDF
**P0; Phase 4; Needs:** K.15; **Code:** `excellent-core::notebook`
1. A zip (`zip` crate): `document.pdf`, `notebook.json` (version, pages with template ids, sections, links, audio
   references), `assets/` (audio, images).
2. Open by reading `document.pdf` into memory and opening it with `fz_open_document_with_stream`.
3. Save atomically (K.15); keep the PDF incremental inside the zip when possible.
4. "Export as PDF" writes a plain PDF with templates baked in (N.1) and audio left out (or attached as embedded files
   on request, O.25).
5. Document the format in `docs/formats/expdf.md`.

**Done when:** a notebook round-trips; its exported PDF opens everywhere.
**Links:** zip crate <https://github.com/zip-rs/zip2>

#### Ph4: 18/44 | N.5: Infinite canvas pages and whiteboards
**P1; Phase 4; Needs:** N.4, H.1; **Code:** `excellent-features/notebook/infinite.rs`
1. A page kind with no fixed size: content coordinates in a large space; the page box grows to the content's bounds
   on save.
2. Pan and zoom freely; minimap (R.17) for orientation.
3. Export to PDF as one large page, or split into tiles of a chosen paper size.

**Done when:** drawing far from the origin keeps full precision; export keeps every stroke.

#### Ph4: 19/44 | N.6: Typed text blocks
**P1; Phase 4; Needs:** N.4, U.8; **Code:** `excellent-features/notebook/textblocks.rs`
1. Blocks drawn and edited inside the canvas (NSH views can't sit over the canvas on Windows, §7.3): shaping, layout
   and editing with `cosmic-text`; input methods through U.8.
2. Block kinds: paragraph, heading, list, checkbox, code, table.
3. Saved as FreeText annotations with rich contents (`pdf_set_annot_rich_contents`) plus the block structure in the
   sidecar.

**Done when:** blocks show readable text in other readers and full structure here.
**Links:** cosmic-text <https://github.com/pop-os/cosmic-text>

#### Ph4: 20/44 | N.7: Images, stickers and sticky notes on pages
**P1; Phase 4; Needs:** A.20; **Code:** `excellent-features/notebook/media.rs`
1. Insert images (file, clipboard, camera on tablets) as image stamps (`pdf_set_annot_stamp_image`); stickers come
   from a pack (K.21) and are inserted the same way.
2. Move, resize, rotate, crop with A.13 handles.

**Done when:** images show in other readers at the same size and place.

#### Ph4: 21/44 | N.8: Covers and sections
**P1; Phase 4; Needs:** N.4, B.1; **Code:** `excellent-features/notebook/sections.rs`
1. Cover page (template or image); sections as outline entries (B.1) plus a colored tab strip beside the page.
2. Reorder sections by drag (moves their pages with `pdf_rearrange_pages`).

**Done when:** sections export as PDF bookmarks.

#### Ph4: 22/44 | N.9: Links between pages
**P1; Phase 4; Needs:** R.7; **Code:** `excellent-features/notebook/links.rs`
1. Select text or a region → "Link to page…" → a Link annotation with `pdf_create_link` and an internal destination.
2. Links to other notebooks and documents use deep links (F.13).

**Done when:** links work in exported PDFs.

#### Ph4: 23/44 | N.10: Quick note from a global shortcut
**P1; Phase 4; Needs:** N.4, K.19; **Code:** `excellent-app::quicknote`
1. A system-wide shortcut registered with NSH's `register_global_shortcut` (v0.5.0, `nsh_core::element::desktop`)
   opens a small window (`open_window`) with a new page in an "Inbox" notebook. If the shortcut is taken (the call
   answers false), say so in settings and let the user pick another.
2. Works when the main window is closed: the app keeps a tray icon (`set_tray`) only if the user enables it.

**Done when:** the shortcut opens a writable page within 300 ms.
**Links:** NSH commit "Give Rust apps the tray and global shortcuts" <https://github.com/longbridge/nsh/pull/1562>

#### Ph5: 20/45 | N.11: Planners and calendars
**P2; Phase 5; Needs:** N.12, N.9; **Code:** `excellent-features/notebook/planner.rs`
1. Generate a planner for a chosen year: year, month, week and day pages with links between them (N.9).
2. Optional due dates from S.14 shown on day pages.

**Done when:** a generated year planner's links all work in other readers.

#### Ph5: 21/45 | N.12: Template packs
**P2; Phase 5; Needs:** K.21, N.1; **Code:** `packs/templates/`
1. Packs of template parameter files and PDF backgrounds; browse, preview and install from the notebook dialog.

**Done when:** installing and removing a pack updates the template list without a restart.

#### Ph5: 22/45 | N.13: Version history per notebook
**P2; Phase 5; Needs:** K.14, N.4; **Code:** `excellent-features/notebook/history.rs`
1. Keep snapshots (incremental PDF revisions plus the operation log) inside the `.expdf`; thin them out with age.
2. Browse versions with page previews; restore the whole notebook or copy a page from an old version.

**Done when:** restoring yesterday's version works and is itself undoable.

#### Ph4: 24/44 | N.14: Markdown editing with math and Mermaid
**P1 (parity); Phase 4; Needs:** K.5; **Code:** `excellent-features/markdown`
1. Source editing with NSH `Editor`; preview in NSH `web_view` (WebView2, WKWebView, WebKitGTK) using cmark-gfm HTML
   plus KaTeX and Mermaid scripts shipped as a pack (K.21), as Enhanced does with WebView2.
2. Table of contents from headings (GitHub slug rules), as Enhanced's `src/MarkdownToc.cpp`.
3. Without the web view (missing runtime), fall back to MuPDF's own Markdown rendering without math and diagrams.
   NSH v0.5.0 tells the app when the platform cannot show a web view; use that signal to switch to the fallback.
4. Serve the preview and the KaTeX / Mermaid pack through an app URL scheme (NSH v0.5.0 lets web views load the
   app's own schemes), so no `file://` access is needed; use `evaluate` for scroll sync and the heading list.

**Done when:** a README with math and a Mermaid diagram previews correctly on all three desktops.
**Links:** NSH web view `C:\dev\nsh-src\website\docs\web-view-and-video.md`; KaTeX <https://katex.org>;
Mermaid <https://mermaid.js.org>

### 27.9 Audio and lectures (L)

#### Ph4: 25/44 | L.1: Record audio while writing
**P0; Phase 4; Needs:** H.9, N.4; **Code:** `excellent-features/audio`
1. Capture with `cpal`; encode Opus (libopus through the `opus` crate) in Ogg; store in the notebook's `assets/` or a
   sidecar next to a PDF.
2. Every stroke and annotation already has a timestamp (H.9); store the recording's start time so each one maps to an
   offset.
3. Playback (`cpal` output) highlights strokes as they were written; tapping a stroke or word seeks there.
4. Ask for microphone permission where the OS requires it (macOS, iPad, Android).

**Done when:** tapping a stroke written 10 minutes into a lecture plays from that moment (±0.5 s).
**Links:** cpal <https://github.com/RustAudio/cpal>; opus crate <https://github.com/SpaceManiac/opus-rs>

#### Ph5: 23/45 | L.2: Offline transcription
**P1; Phase 5; Needs:** L.1, K.21, F.3; **Code:** `excellent-features/audio/transcribe.rs`
1. whisper.cpp through `whisper-rs`; models as packs (tiny to medium, user's choice).
2. Run at background priority after recording; store segments with timestamps; link each segment to the page that was
   open at that time.
3. Index transcripts in FTS5 (F.3); show them in a side panel synced to playback.

**Done when:** searching a spoken word finds the moment and the page.
**Links:** whisper.cpp <https://github.com/ggml-org/whisper.cpp>; whisper-rs
<https://github.com/tazz4843/whisper-rs>

#### Ph4: 26/44 | L.3: Import slides and condense to notes
**P1; Phase 4; Needs:** N.4; **Code:** `excellent-features/notebook/slides.rs`
1. Pick a slide PDF; choose 1, 2 or 3 slides per page with lined note space (Cornell-style option).
2. Build each new page with `pdf_page_write` and run the source pages into it scaled (vector, not images); add with
   `pdf_add_page` / `pdf_insert_page`.

**Done when:** text in the condensed slides stays selectable and searchable.

#### Ph7: 5/32 | L.4: Screen and pen recording to video
**P2; Phase 7; Needs:** L.1; **Code:** `excellent-features/audio/video.rs`
1. Record canvas frames and audio; encode with the platform encoder (Media Foundation, AVAssetWriter, GStreamer) to
   MP4.
2. Playback with NSH `video`.

**Done when:** a 30-minute recording plays in the OS video player with audio in sync.

### 27.10 Study and language (S)

#### Ph4: 27/44 | S.1: Flashcards from highlights, notes and lasso selections
**P0; Phase 4; Needs:** A.1, H.6, S.4; **Code:** `excellent-features/study/cards.rs`
1. Card model in `library.db`: front, back, source anchor (K.28), deck, tags, media.
2. "Make card" from a highlight (front = text, back = note), from a note, or from a lasso selection (front = region
   image).
3. Every card links back to its page.

**Done when:** a card made from a highlight opens its source page from the review screen.

#### Ph4: 28/44 | S.2: Image occlusion
**P0; Phase 4; Needs:** S.1; **Code:** as S.1
1. Draw boxes over a page region or figure; each box becomes one card hiding that box (or all boxes, "hide all,
   guess one").

**Done when:** occlusion cards export to Anki (S.11) as image-occlusion notes.

#### Ph4: 29/44 | S.3: Cloze deletions
**P0; Phase 4; Needs:** S.1; **Code:** as S.1
1. Select words inside a highlight → cloze; `{{c1::…}}` syntax compatible with Anki.

**Done when:** cloze cards round-trip through Anki export and import.

#### Ph4: 30/44 | S.4: Spaced repetition (FSRS)
**P0; Phase 4; Needs:** K.20; **Code:** `excellent-features/study/srs.rs`
1. One scheduler for cards and vocabulary: the `fsrs` crate (FSRS-rs); review log table; optimizer run on demand.
2. Migrate Enhanced's vocabulary scheduling state (`SumatraPDF-vocabulary.json`).

**Done when:** scheduling matches the FSRS reference implementation on its test vectors.
**Links:** fsrs-rs <https://github.com/open-spaced-repetition/fsrs-rs>

#### Ph4: 31/44 | S.5: Daily review on the home page
**P0; Phase 4; Needs:** S.4, F.17; **Code:** `excellent-features/study/review.rs`
1. Due count on the home page; review screen with keyboard grading (1–4); cards show their source crop.

**Done when:** a review session of 100 cards never waits on rendering (crops cached).

#### Ph4: 32/44 | S.6: Offline dictionary
**P0 (parity); Phase 4; Needs:** K.21, K.13; **Code:** `excellent-features/dictionary`
1. Port Enhanced's dictionary (`src/OfflineDictionary.cpp`, `src/KaikkiDictionary.cpp`): Kaikki packs with
   `manifest.json`, `index.bin` (u64 offsets, hash-collision filtering) and `records.jsonl`.
2. Port its parsing and index tests first.
3. Dictionaries become packs (K.21); tap or double-click a word to look it up (word boundaries from K.13).

**Done when:** the ported tests pass; lookups take under 10 ms.
**Links:** Kaikki <https://kaikki.org>

#### Ph4: 33/44 | S.7: Vocabulary practice
**P0 (parity); Phase 4; Needs:** S.4, S.6; **Code:** `excellent-features/vocabulary`
1. Port Enhanced's vocabulary library and practice dialogs (`src/Vocabulary.cpp`, `src/VocabularyDialog.cpp`) onto
   FSRS (S.4).
2. Import the user's existing vocabulary file.

**Done when:** an imported Enhanced vocabulary keeps every word and its due dates.

#### Ph4: 34/44 | S.8: Study export
**P0 (parity); Phase 4; Needs:** A.14; **Code:** `excellent-features/study/export.rs`
1. Port Enhanced's study export (`src/StudyExport.cpp`): highlights, notes and vocabulary to its existing formats.

**Done when:** output matches Enhanced's for the same document.

#### Ph6: 3/13 | S.9: Quizzes from a chapter
**P1; Phase 6; Needs:** I.3, S.1; **Code:** `excellent-features/study/quiz.rs`
1. Generate questions (multiple choice, short answer) from a page range with the AI pipeline; every question cites its
   source passage (I.3).
2. The user edits and accepts questions before they become cards; nothing is added silently.

**Done when:** every generated question links to a real passage (I.12 checks).

#### Ph4: 35/44 | S.10: Exam mode
**P1; Phase 4; Needs:** S.1, H.4; **Code:** `excellent-features/study/exam.rs`
1. Hides answers, notes and chosen annotation categories; tape (H.4) covers definitions; a timer; reveal one at a time.

**Done when:** leaving exam mode restores the exact previous visibility.

#### Ph4: 36/44 | S.11: Anki import and export
**P1; Phase 4; Needs:** S.1; **Code:** `excellent-features/study/anki.rs`
1. `.apkg` is a zip with a SQLite collection and media; read and write it with `rusqlite` and `zip`.
2. Map basic, cloze and image-occlusion note types; keep source links in a field.

**Done when:** an exported deck imports into Anki desktop, and Anki's export imports here.
**Links:** Anki <https://github.com/ankitects/anki>; genanki (format reference) <https://github.com/kerrickstaley/genanki>

#### Ph4: 37/44 | S.12: Goals, timer, statistics and streaks
**P1; Phase 4; Needs:** S.4; **Code:** `excellent-features/study/stats.rs`
1. Daily goals (cards, pages, minutes); focus timer; charts with NSH `Plot`; streaks with a gentle celebration
   (reduced-motion aware).

**Done when:** statistics are computed from the review log and reading time, not stored twice.

#### Ph5: 24/45 | S.13: Summaries from your highlights
**P1; Phase 5; Needs:** A.14; **Code:** `excellent-features/study/summary.rs`
1. Without AI: collect highlights and notes in reading order into a Markdown summary grouped by heading (B.8).
2. With AI (I.4): optional condensed version that cites each highlight.

**Done when:** the non-AI summary works offline with no pack.

#### Ph5: 25/45 | S.14: Assignments with due dates
**P2; Phase 5; Needs:** F.6; **Code:** `excellent-features/study/assignments.rs`
1. Attach a due date and checklist to a document or page range; reminders through OS notifications (K.19); calendar
   with NSH `Calendar`.

**Done when:** reminders fire at the right time after a restart.

#### Ph5: 26/45 | S.15: Language learning
**P2; Phase 5; Needs:** S.6, S.7, U.5; **Code:** `excellent-features/language`
1. Tap a word: dictionary entry, pronunciation (TTS, U.5), "add to deck".
2. Parallel translation: a second pane (V.1) showing the translated text of the current page, paragraph-aligned (S.16).

**Done when:** a word goes from page to deck in two taps.

#### Ph5: 27/45 | S.16: Translate selection
**P1 (parity); Phase 5; Needs:** K.13; **Code:** `excellent-features/translate`
1. Port Enhanced's selection translation (`src/SelectionTranslate.cpp`, `src/TranslationLangs.cpp`): online providers
   only after consent.
2. Offline: Firefox Translations (Bergamot) models as packs.

**Done when:** offline translation works with the network disabled.
**Links:** Firefox Translations models <https://github.com/mozilla/translations>

### 27.11 Teaching and presenting (G)

#### Ph4: 38/44 | G.1: Present mode
**P0; Phase 4; Needs:** R.19; **Code:** `excellent-features/present`
1. Full screen, one page per screen, fit page; arrow keys, clicker keys (PgUp / PgDn, B for black screen, W for white).
2. Present-mode toolbar (§10.2) shown on pointer movement only.

**Done when:** a USB clicker works without setup.

#### Ph4: 39/44 | G.2: Laser with trails; G.3: Spotlight; G.4: Magnifier
**P0; Phase 4; Needs:** G.1, K.8; **Code:** `excellent-features/present/pointers.rs`
1. All three are canvas overlays (no re-render): laser dot with a fading trail (port Enhanced's laser), spotlight
   (darken all but a circle or rectangle), magnifier (lens that renders the region at higher zoom from tiles).

**Done when:** all three follow the pointer in the same frame.

#### Ph4: 40/44 | G.5: Pen on slides and fading ink
**P0; Phase 4; Needs:** G.1, H.1; **Code:** as G.2
1. Draw on slides without saving to the file by default; "fading ink" disappears after a few seconds; "keep" saves
   it as ink (H.9).

**Done when:** temporary ink never modifies the file unless kept.

#### Ph4: 41/44 | G.6: Whiteboard page
**P0; Phase 4; Needs:** G.1, N.5; **Code:** as G.2
1. Insert a blank (or grid) page temporarily during a presentation; optionally save it into a notebook afterwards.

**Done when:** the original file is unchanged after presenting.

#### Ph4: 42/44 | G.7: Curtain reveal
**P0; Phase 4; Needs:** G.1; **Code:** as G.2
1. A curtain covers the page and slides down (or across) as the presenter reveals it line by line.

**Done when:** keyboard and pen can move the curtain.

#### Ph4: 43/44 | G.8: Presenter view
**P0; Phase 4; Needs:** G.1, K.19; **Code:** `excellent-features/present/presenter.rs`
1. A second window on the second display: current and next page, notes (page notes from annotations or a notes
   file), timer, clock.
2. The audience window shows only the slide; both stay in sync.

**Done when:** works with any two monitors and survives a display being unplugged.

#### Ph7: 6/32 | G.9: Grading stamps
**P1; Phase 7; Needs:** A.20; **Code:** `excellent-features/grading`
1. Quick stamps ✓ ✗ ? and score stamps (+1, −½…); each stamp carries a value in a private key.

**Done when:** stamps show in other readers as normal stamps.

#### Ph7: 7/32 | G.10: Rubric panel and scores
**P1; Phase 7; Needs:** G.9; **Code:** as G.9
1. Rubric editor (criteria, levels, points); per-page and total scores summed from stamps and rubric choices; final
   score stamp on page 1.

**Done when:** totals recalculate when a stamp is changed or removed.

#### Ph7: 8/32 | G.11: Comment bank
**P1; Phase 7; Needs:** A.4; **Code:** as G.9
1. Saved comments with shortcuts; insert as sticky notes or text boxes; usage counts.

**Done when:** the bank imports and exports as JSON.

#### Ph7: 9/32 | G.12: Batch open and export
**P1; Phase 7; Needs:** G.10, F.6; **Code:** as G.9
1. Open a folder of submissions as a workspace (W.10); next / previous student; export all graded PDFs and a CSV of
   scores.

**Done when:** 50 submissions export with matching CSV totals.

#### Ph7: 10/32 | G.13: Worksheet maker
**P1; Phase 7; Needs:** O.11, A.22; **Code:** `excellent-features/worksheets`
1. Add fillable fields (text, checkbox, choice). MuPDF has a helper only for signature fields, so write the field
   dictionaries (`/FT`, `/T` from `pdf_create_field_name`, `/Rect`, `/MK`) with `pdf_dict_put`, add them to the page's
   `/Annots` and the AcroForm `/Fields`, then style with `pdf_field_set_*` and `pdf_update_annot`. PdfCraft creates
   every field type and is worth reading first (Appendix C.5).
2. An "answer key" layer (A.22), hidden when exporting the student copy.

**Done when:** the student copy has working fields and no answers.

#### Ph9: 2/13 | G.14: Live classroom
**P2; Phase 9; Needs:** Y.4; **Code:** `excellent-features/classroom`
1. Teacher shares the current page over the LAN (mDNS discovery) or a self-hosted relay; students follow; no
   accounts; join by code.

**Done when:** 30 students follow page changes within 300 ms on a LAN.

#### Ph9: 3/13 | G.15: Quick polls
**P2; Phase 9; Needs:** G.14; **Code:** as G.14
1. Teacher asks a multiple-choice question; students answer; results as a chart (NSH `Plot`).

**Done when:** results are anonymous by default.

### 27.12 Find: search, library and citations (F)

#### Ph1: 53/60 | F.1: Find in document
**P0; Phase 1; Needs:** K.13; **Code:** `excellent-features/search/find.rs`
1. Find bar (Ctrl+F): case, whole word, diacritics-insensitive; results stream in page order from workers using
   `fz_search_stext_page_cb` on K.13 text.
2. Hits drawn in the canvas; result list in the sidebar with context; F3 / Shift+F3.

*Built (2026-11-15):* steps 1 and 2, with matching done in Rust over K.13's text instead of
`fz_search_stext_page_cb`, so whole words and accent folding are options (DD5). The result list shows the first 500
hits; the count and F3 reach all of them.

**Done when:** first hit on a 1,000-page book shows within 100 ms when the page is near; the UI never waits.

#### Ph5: 28/45 | F.2: Search a folder or the library
**P0; Phase 5; Needs:** F.3; **Code:** `excellent-features/search/global.rs`
1. Query FTS5 for indexed files; scan unindexed files in the background; results stream while indexing continues.

**Done when:** results appear within 100 ms for indexed content (§13 budget).

#### Ph5: 29/45 | F.3: Full-text index
**P0; Phase 5; Needs:** K.20, K.13; **Code:** `excellent-store::fts`
1. FTS5 tables for page text, annotation text, handwriting (H.23), transcripts (L.2) and notes, with
   `unicode61 remove_diacritics 2`, and a trigram table for substring search.
2. An indexer at background priority; pauses while the user scrolls or writes (§7.4).
3. Store page numbers and quads so hits open at the exact spot.

**Done when:** indexing 1,000 pages takes under a minute on the reference machine and never stalls the UI.
**Links:** SQLite FTS5 <https://www.sqlite.org/fts5.html>

#### Ph5: 30/45 | F.4: Search operators
**P1; Phase 5; Needs:** F.3; **Code:** `excellent-features/search/query.rs`
1. Parser for `tag:`, `color:`, `author:`, `after:`, `page:>100`, AND / OR / NOT, phrases, wildcards, fuzzy (`~`),
   regex (`/…/`, run on candidate pages only).
2. Translate to FTS5 plus SQL filters; property tests for the parser.

**Done when:** every operator has a test; malformed queries show a helpful message.

#### Ph5: 31/45 | F.5: Saved searches and smart folders
**P1; Phase 5; Needs:** F.4, F.6; **Code:** as F.4
1. Save a query by name; smart folders are saved searches shown in the library tree; counts update live.

**Done when:** smart folders update within a second of a new match.

#### Ph5: 32/45 | F.6: Library: collections, tags, read status
**P1; Phase 5; Needs:** K.20; **Code:** `excellent-features/library`
1. Tables for collections (nested), tags, read status (unread, reading, done) and progress.
2. Library view with NSH `DataTable` and covers; filters; drag files into collections.

**Done when:** 50,000 documents browse smoothly.

#### Ph5: 33/45 | F.7: Metadata extraction
**P1; Phase 5; Needs:** F.6; **Code:** `excellent-features/library/metadata.rs`
1. From document info and XMP; DOI, arXiv id and ISBN found by pattern in the first pages.
2. Optional online lookup (consent): Crossref for DOI, the arXiv API, Open Library for ISBN.

**Done when:** DOIs are found in 95 % of the paper corpus.
**Links:** Crossref REST API <https://www.crossref.org/documentation/retrieve-metadata/rest-api/>; arXiv API
<https://info.arxiv.org/help/api/index.html>; Open Library API <https://openlibrary.org/developers/api>

#### Ph5: 34/45 | F.8: Duplicate detection
**P1; Phase 5; Needs:** F.6; **Code:** as F.7
1. Exact duplicates by content hash; near duplicates by DOI or by text shingles (MinHash) of the first pages.

**Done when:** a preprint and its published version are flagged as related, not identical.

#### Ph5: 35/45 | F.9: Folder watching
**P1; Phase 5; Needs:** F.6; **Code:** `excellent-features/library/watch.rs`
1. Watch chosen folders with the `notify` crate; add, update and remove library entries; index new files at idle.

**Done when:** a file saved into a watched folder appears within 2 s.
**Links:** notify <https://github.com/notify-rs/notify>

#### Ph5: 36/45 | F.10: Citations
**P1; Phase 5; Needs:** F.7; **Code:** `excellent-features/citations`
1. Detect DOI and arXiv references in a document's reference list; fetch metadata (consent).
2. Export BibTeX and RIS for a document, a selection of references or a collection; copy a formatted citation (CSL
   styles through a CSL processor such as `hayagriva`).

**Done when:** exported BibTeX compiles with biber for the corpus papers.
**Links:** hayagriva <https://github.com/typst/hayagriva>; CSL styles <https://citationstyles.org>

#### Ph5: 37/45 | F.11: Zotero integration
**P1; Phase 5; Needs:** F.10; **Code:** `excellent-features/citations/zotero.rs`
1. Read the library through Zotero 7's local API on `localhost:23119` when Zotero runs; otherwise the web API with the
   user's key (in the OS key store).
2. Open a Zotero item's PDF here; send annotations back as notes (optional).

**Done when:** a Zotero item opens with its metadata in the inspector.
**Links:** Zotero web API (the local API follows it) <https://www.zotero.org/support/dev/web_api/v3/basics>

#### Ph5: 38/45 | F.12: Notes with backlinks and `[[links]]`
**P1; Phase 5; Needs:** F.3, N.14; **Code:** `excellent-features/notes`
1. Markdown notes in `library.db` (or a folder of `.md` files, user's choice); `[[note]]`, `[[doc#page]]` and
   annotation links; a backlinks panel.
2. Edit with NSH `RichEditor` or `Editor`.

**Done when:** renaming a note updates every link to it.

#### Ph5: 39/45 | F.13: Export to Markdown and Obsidian with deep links
**P1; Phase 5; Needs:** F.12; **Code:** `excellent-features/notes/export.rs`
1. Deep links: register `excellentpdf://open?file=…&page=…&annot=…` with the OS.
2. Export notes and highlights to a folder (Obsidian vault) with those links.

**Done when:** clicking a link in Obsidian opens the right page and annotation.

#### Ph9: 4/13 | F.14: Graph view
**P2; Phase 9; Needs:** F.12; **Code:** `excellent-features/notes/graph.rs`
1. Force-directed graph of notes, documents and links, drawn with NSH's canvas element.

**Done when:** 5,000 nodes stay interactive.

#### Ph7: 11/32 | F.15: PDF compare
**P2; Phase 7; Needs:** K.13, V.8; **Code:** `excellent-features/compare`
1. Text diff by page with word-level changes (`similar` crate); layout diff by rendering both and comparing pixels;
   annotation diff by `/NM` id.
2. Show side by side with synced scrolling (V.8) and change markers; report as PDF or HTML.

**Done when:** a one-word change in a 300-page book is found and shown.
**Links:** similar <https://github.com/mitsuhiko/similar>

#### Ph7: 12/32 | F.16: Table and figure extraction
**P2; Phase 7; Needs:** K.13; **Code:** `excellent-features/extract`
1. Tables: `FZ_STEXT_TABLE_HUNT` and `fz_find_table_within_bounds`; export CSV (`fz_new_csv_writer`) or Markdown.
2. Figures: image blocks plus vector regions with their captions; export PNG or SVG.

**Done when:** tables in the corpus papers export with the right rows and columns in 90 % of cases.

#### Ph1: 54/60 | F.17: Home page
**P0; Phase 1; Needs:** R.10; **Code:** `excellent-features/home`
1. Recent and pinned files with thumbnails, continue reading, workspaces, daily review (S.5) and a search box.

*Built (2026-11-15):* everything but the daily review, which comes with S.5. It is what a pane with no tab shows; the
thumbnails are the files' first pages, rendered in the background and cached by path, size and date.

**Done when:** the home page paints within the cold-start budget.

### 27.13 Excerpt (research) canvas (X)

#### Ph9: 5/13 | X.1: Excerpts on an infinite canvas
**P1; Phase 9; Needs:** K.28, N.5; **Code:** `excellent-features/research-canvas`
1. A board document (`.exboard`, JSON plus assets) holding items: text excerpts, region crops, notes, images.
2. Drag a selection or region from a page onto the board (V.1 puts them side by side); each item stores its anchor
   (K.28) and jumps back on click.
3. Draw the board with the page canvas in "board" mode (shared tiles for crops) or NSH's canvas element.

**Done when:** an excerpt still jumps to the right place after the source is re-typeset.

#### Ph9: 6/13 | X.2: Connect, group and label
**P1; Phase 9; Needs:** X.1; **Code:** as X.1
1. Arrows between items with labels; groups with titles and colors; argument-map item kinds (claim, support,
   objection).

**Done when:** moving a group moves its items and keeps arrows attached.

#### Ph9: 7/13 | X.3: Evidence chains across documents
**P1; Phase 9; Needs:** X.2; **Code:** as X.1
1. A "chain" view lists connected items in order with their sources, across documents.

**Done when:** a chain exports with every source cited.

#### Ph9: 8/13 | X.4: Export a board
**P2; Phase 9; Needs:** X.1; **Code:** as X.1
1. Export as Markdown (with deep links, F.13), PNG or SVG, PDF appendix, or an outline.

**Done when:** each export opens correctly in its target app.

### 27.14 PDF tools (O)

#### Ph7: 13/32 | O.1: Page organizer
**P0; Phase 7; Needs:** R.5, K.14; **Code:** `excellent-features/pages`
1. A grid of thumbnails (multi-select) with insert, delete, rotate, reorder, extract and duplicate.
2. Reorder and delete with `pdf_rearrange_pages`; delete ranges with `pdf_delete_page_range`; rotate by setting the
   page's `/Rotate`; insert with `pdf_add_page` / `pdf_insert_page`; extract by copying pages into a new document
   with a graft map (`pdf_graft_mapped_page`).
3. Fix up the outline and personal bookmarks after page changes (§11.14, step 5); check first what
   `pdf_rearrange_pages` already does.
4. Edit page labels (`pdf_set_page_labels`, `pdf_delete_page_labels`).

**Done when:** every operation is one undo step; links, outline, form fields and annotations survive extraction
(compare with PdfCraft's checklist, Appendix C.1).

#### Ph7: 14/32 | O.2: Split and merge
**P0; Phase 7; Needs:** O.1; **Code:** `excellent-features/pages/combine.rs`
1. Merge files (and page ranges) in a chosen order with `pdf_graft_mapped_page`; one outline entry per file (B.8);
   port Enhanced's merge dialog ideas (`src/MergePdf.cpp`).
2. Split by page count, by outline level, by size, or at chosen pages.
3. Same operations in the CLI (E.1).

**Done when:** merging 100 files keeps every file's outline and links.

#### Ph7: 15/32 | O.3: Crop pages
**P0; Phase 7; Needs:** O.1; **Code:** `excellent-features/pages/crop.rs`
1. Draw a crop rectangle; apply to this page, all pages, odd or even pages; "auto-crop to content" from display-list
   bounds; write with `pdf_set_page_box`.

**Done when:** cropped pages show the same in Acrobat; uncrop restores the original box.

#### Ph5: 40/45 | O.4: OCR to searchable PDF
**P1; Phase 5; Needs:** K.21, K.22; **Code:** `excellent-features/ocr`, `excellent-ocr` helper
1. Fast path, no download: the OS OCR (Windows `Windows.Media.Ocr`, Apple Vision) on rendered pages.
2. Full path, as a pack: a helper process built with MuPDF plus Tesseract and Leptonica that writes a text layer with
   `fz_new_ocr_device` or `fz_new_pdfocr_writer`; language data as packs. *Open (2026-11-13):* Tesseract and
   Leptonica are now built into the core MuPDF library on every target (`dev_decisions.md` DD1; upstream SumatraPDF
   vendors them in `ext/`), so a separate helper may not be needed; whether the code stays in the core installer
   depends on the 15 MB budget and is the maintainer's call (D8).
3. Add the invisible text layer to the original pages (not a re-render), so the page stays crisp.
4. Detect scanned pages automatically and offer OCR; index results (F.3).

**Done when:** OCR'd pages are searchable and selectable in other readers; the original images are unchanged.
**Links:** Tesseract <https://github.com/tesseract-ocr/tesseract>; tessdata <https://github.com/tesseract-ocr/tessdata_fast>;
Windows OCR <https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr>

#### Ph7: 16/32 | O.5: Edit text runs
**P1; Phase 7; Needs:** K.13, O.7; **Code:** `excellent-features/edit-text`
1. Click a text line → edit box drawn in the canvas with the line's text and font.
2. On commit: remove the old run with a text-only redaction of its quads (`pdf_redact_page` with images and line art
   kept), then write the new text into the page contents (`pdf_page_write`) with the same font if its embedded subset
   has the glyphs, otherwise a close substitute added with `pdf_add_simple_font` or `pdf_add_cjk_font`.
3. Warn when the font can't be matched; never edit signed documents without the warning of O.16.

**Done when:** editing a word in a Latin and a CJK document keeps position, size and color.

#### Ph7: 17/32 | O.6: Replace images
**P1; Phase 7; Needs:** K.13; **Code:** `excellent-features/edit-image`
1. Hit-test image blocks (`FZ_STEXT_PRESERVE_IMAGES`); find the image XObject in the page resources; replace its
   stream with a new image from `pdf_add_image`, keeping the placement matrix.
2. Export an image at native resolution.

**Done when:** a replaced image sits in the same box in other readers.

#### Ph7: 18/32 | O.7: True redaction
**P1; Phase 7; Needs:** A.9, F.4; **Code:** `excellent-features/redact` (§14.3)
1. Mark areas, or search and mark patterns (names, numbers, regex, word lists); review list.
2. Apply with `pdf_apply_redaction` and `pdf_redact_options` (text, images, line art; black boxes on or off).
3. Scrub metadata, XMP, attachments, hidden layers, thumbnails, comments and form values in the areas.
4. Save as a new full file (`do_garbage`, no incremental sections) so old revisions are gone.
5. Verify: reopen, search for every removed string and pattern, show a report; never claim success without it.

**Done when:** the redaction corpus with known secrets passes verification; PdfCraft's verification approach is
checked for ideas (Appendix C.5).

#### Ph7: 19/32 | O.8: Compress
**P1; Phase 7; Needs:** K.15; **Code:** `excellent-features/compress`
1. Presets (screen, ebook, print): downsample and recompress images with `pdf_rewrite_images`; subset fonts with
   `pdf_subset_fonts`; save with `do_garbage`, `do_compress`, `do_use_objstms`, `compression_effort`; optional
   linearize (`do_linear`) for the web.
2. Show size before and after; save as a copy by default.

**Done when:** the output passes `qpdf --check` and renders the same as the input at the preset's resolution.

#### Ph7: 20/32 | O.9: Images to PDF and PDF to images
**P1; Phase 7; Needs:** K.3; **Code:** `excellent-features/convert/images.rs`
1. Images to PDF: open each image with `fz_open_document`, write pages with `fz_new_document_writer` (PDF), page size
   from image DPI or a chosen paper size.
2. PDF to images: `fz_new_pixmap_from_page` at a chosen DPI to PNG or JPEG; pages or ranges; in the CLI too.

**Done when:** a folder of 200 photos becomes one PDF in the right order (natural sort).

#### Ph7: 21/32 | O.10: PDF to Markdown and DOCX
**P1; Phase 7; Needs:** R.16; **Code:** `excellent-features/convert/text.rs`
1. DOCX and ODT with MuPDF's writers (`fz_new_docx_writer`, `fz_new_odt_writer`, using Artifex extract).
2. Markdown from the reflow model (R.16): headings, lists, tables, images as files, math where recoverable.

**Done when:** headings and tables of the paper corpus survive into Markdown.

#### Ph7: 22/32 | O.11: Forms: fill, validate and calculate
**P1; Phase 7; Needs:** K.14, U.8; **Code:** `excellent-features/forms`
1. Enable form JavaScript with `pdf_enable_js` (MuJS, MuPDF's restricted API); show alerts from
   `pdf_set_doc_event_callback` as app dialogs.
2. Walk widgets (`pdf_first_widget`, `pdf_next_widget`); edit text in the canvas (as N.6), set values with
   `pdf_set_text_field_value`, `pdf_set_choice_field_value`, `pdf_toggle_widget`; recalculate with
   `pdf_calculate_form`.
3. Tab order; required-field and format validation; "highlight fields".
4. XFA forms: show the AcroForm fallback where present and say so; full XFA is a gap (Appendix C.1).

**Done when:** the forms corpus fills, calculates and saves; filled values show in Acrobat.

#### Ph7: 23/32 | O.12: Save and reuse form values
**P1; Phase 7; Needs:** O.11; **Code:** `excellent-features/forms/profiles.rs`
1. Save values by field name to a profile (encrypted in `library.db`, key in the OS key store); suggest them in
   other forms with matching names; export and import FDF / XFDF values (A.24).

**Done when:** a profile fills a second form with matching field names.

#### Ph7: 24/32 | O.13: Flatten forms
**P1; Phase 7; Needs:** O.11; **Code:** as A.25
1. `pdf_bake_document` with `bake_widgets = 1` on a copy.

**Done when:** the flattened copy looks the same and has no fields.

#### Ph7: 25/32 | O.14: Sign documents
**P1; Phase 7; Needs:** O.11, K.19; **Code:** `crates/excellent-sign`
1. A Rust signer behind MuPDF's `pdf_pkcs7_signer` callbacks: CMS SignedData with the RustCrypto `cms` crate,
   PAdES baseline B-B (`ETSI.CAdES.detached`, signing-certificate-v2 attribute).
2. Keys: PKCS #12 files, and OS stores (Windows CNG through the `windows` crate, macOS Keychain, PKCS #11 tokens
   through `cryptoki`).
3. Sign a new or existing field with `pdf_create_signature_widget` and `pdf_sign_signature` (appearance, reason,
   location, date).
4. Optional timestamp (B-T) from a time-stamp authority (RFC 3161), after consent.
5. Port the user flow from Enhanced's `src/PdfSign.cpp` and `src/SignDocumentDialog.cpp`; it replaces patch `0003`.

**Done when:** signatures validate in Acrobat and with `pdfsig`; a pictured "signature" is never called a digital
signature in the UI.
**Links:** RustCrypto formats (cms, x509-cert) <https://github.com/RustCrypto/formats>; cryptoki
<https://github.com/parallaxsecond/rust-cryptoki>; RFC 5652 <https://www.rfc-editor.org/rfc/rfc5652>; RFC 3161
<https://www.rfc-editor.org/rfc/rfc3161>; ETSI EN 319 142 (PAdES) <https://www.etsi.org/standards>

#### Ph7: 26/32 | O.15: Validate signatures and trust
**P1; Phase 7; Needs:** O.14; **Code:** `crates/excellent-sign::verify`
1. A Rust `pdf_pkcs7_verifier`; check with `pdf_check_digest` and `pdf_check_certificate`; read signer details with
   `pdf_signature_get_signatory`.
2. Chain building with `x509-cert`; trust from the OS store and the EU Trusted List (port Enhanced's
   `src/EutlTrust.cpp`); revocation from embedded `/DSS` data, or online after consent.
3. Signature panel: valid, invalid, unknown signer, modified after signing (`pdf_signature_incremental_change_since_signing`).
4. "View signed version" opens the revision covered by the byte range (`pdf_signature_byte_range`).

**Done when:** results match Acrobat's on the reference signed files (§16).
**Links:** EU Trusted List browser <https://eidas.ec.europa.eu/efda/tl-browser/>

#### Ph7: 27/32 | O.16: Signature-safe editing
**P1; Phase 7; Needs:** O.15; **Code:** `excellent-core::save`
1. Before any edit to a signed document, warn which signatures the change would break; allowed changes (per `/DocMDP`
   and `/FieldMDP`) pass without a warning.
2. Always save signed documents incrementally, keeping signed revisions intact.

**Done when:** adding a comment to a document certified for comments keeps the certification valid.

#### Ph7: 28/32 | O.17: Bates numbering
**P2; Phase 7; Needs:** O.18; **Code:** `excellent-features/stamp-pages/bates.rs`
1. Prefix, start number, digits, suffix, position; across a set of files in order; written as page content (O.18).

**Done when:** numbering continues correctly across 20 files.

#### Ph7: 29/32 | O.18: Headers, footers and watermarks
**P2; Phase 7; Needs:** O.1; **Code:** `excellent-features/stamp-pages`
1. Build the new content with `pdf_page_write` (text, image or page as a watermark, opacity, rotation); wrap the old
   contents in `q … Q`; append the new stream to `/Contents`; merge `/Resources`.
2. Mark added content as an optional content group named for the tool, so it can be found and removed later.

**Done when:** adding then removing a watermark returns the original rendering.

#### Ph7: 30/32 | O.19: Permissions and encryption
**P2; Phase 7; Needs:** K.15; **Code:** `excellent-features/security`
1. Save with `do_encrypt` (AES-256), user and owner passwords, `permissions` bits; remove security with the password.
2. Honour permissions when opening other files (copy, print, edit), with a clear message.

**Done when:** encrypted files open in Acrobat with the same permissions (Appendix C.4).

#### Ph7: 31/32 | O.20: PAdES long-term validation
**P2; Phase 7; Needs:** O.15; **Code:** `crates/excellent-sign::ltv`
1. Add a `/DSS` dictionary with certificates, OCSP responses and CRLs (plus `/VRI`), incrementally; then a document
   timestamp for B-LTA.

**Done when:** Acrobat shows "LTV enabled" for the signatures.

#### Ph8: 3/3 | O.21: Camera scan on tablets
**P2; Phase 8; Needs:** O.9, O.4; **Code:** `excellent-features/scan`
1. iPad: VisionKit document camera; Android: ML Kit document scanner; both return cropped, de-skewed page images.
2. Images to PDF (O.9), then OCR (O.4).

**Done when:** a scanned 10-page document is searchable.
**Links:** ML Kit document scanner <https://developers.google.com/ml-kit/vision/doc-scanner>; VisionKit
<https://developer.apple.com/documentation/visionkit>

#### Ph1: 55/60 | O.22: Print
**P0; Phase 1 (Windows), Phase 2 (macOS, Linux); Needs:** K.19; **Code:** `crates/excellent-platform::print`
1. Print dialog with range, copies, scaling (fit, shrink, actual), duplex, color, "print annotations", "print as
   image".
2. Windows: render pages in bands at printer resolution to the printer DC (as SumatraPDF does). macOS:
   `NSPrintOperation` drawing rendered pages. Linux: `GtkPrintOperation`.
3. Print runs on a worker with progress and cancel.

*Built (2026-11-15, Windows; `docs/features/O.22-print.md`):* Ctrl+P opens the system dialog (`PrintDlgExW`: range,
copies, and duplex and colour in the printer's preferences); pages print as images in bands at the printer's
resolution, on a thread of their own, with a progress strip and Cancel. Scaling and "print annotations" are settings
rather than dialog controls (DD6); "print as image" is how every page prints. macOS and Linux stay Phase 2.

**Done when:** printing works on all three desktops before the first public build (Appendix C.4).

#### Ph3: 47/49 | O.23: Screenshots and region snapshots
**P1 (parity); Phase 3; Needs:** K.8; **Code:** `excellent-features/snapshot`
1. Drag a region; render it from the display list at a chosen DPI (not a screen grab); copy, save as PNG, or make a
   card (S.1); port the ideas of Enhanced's `src/Screenshot*.cpp`.

**Done when:** a 600 DPI snapshot of a vector figure is sharp.

*Built (2026-11-25):* `excellent-features::snapshot` (render from the display list at 96 to 600 dpi, capped at 40
megapixels), snapshot mode on the canvas, and the dialog (Copy as image, Save PNG). Enhanced's screenshots grab the
screen; only the quick command and the ready file name carry over. "Make a card" waits for S.1.

#### Ph7: 32/32 | O.24: Color-accurate and print-production view
**P2; Phase 7; Needs:** K.7; **Code:** `excellent-features/prepress` (Appendix D.4)
1. Color management with lcms2 (`fz_enable_icc`); the display's ICC profile from the OS; output intents from
   `pdf_document_output_intent` and `fz_set_default_output_intent` for soft-proofing.
2. Overprint preview through `fz_color_params` (`op`, `opm`) and separations (`fz_page_separations`,
   `fz_set_separation_behavior`, `fz_new_pixmap_from_page_with_separations`). This needs spot rendering enabled in
   MuPDF's `config.h` (`FZ_ENABLE_SPOT_RENDERING`); check Sumatra's build.
3. Separations panel: toggle each ink; ink coverage at the pointer.

**Done when:** overprint and separations match Acrobat's Output Preview on the Ghent Workgroup test suite.
**Links:** Ghent Output Suite <https://gwg.org/ghent-output-suite/>

#### Ph3: 48/49 | O.25: Attachments
**P1; Phase 3; Needs:** K.3, K.23; **Code:** `excellent-features/attachments`
1. List embedded files (the `/EmbeddedFiles` name tree via `pdf_load_name_tree`) and file-attachment annotations; open
   after confirmation (never run them, §14.2); save as; add with `pdf_add_embedded_file` or as an annotation with
   `pdf_set_annot_filespec`.

**Done when:** names with `../` can't write outside the chosen folder (test).

*Built (2026-11-25):* listing (name tree and page annotations), Save, Save all into a folder with safe names, Open
after asking and never for programs, and Add (`pdf_add_embedded_file`, one undoable operation; MuPDF patch `0095`
makes undo work after it). Adding a page's file attachment annotation is not built.

### 27.15 Sync and sharing (Y)

#### Ph9: 9/13 | Y.1: Folder sync with CRDT merge
**P1; Phase 9; Needs:** K.14, K.28; **Code:** `crates/excellent-sync`
1. Each device appends its own operation log file in the sync folder (`.excellent/<doc-id>/<device-id>.oplog`), so
   Dropbox, OneDrive, iCloud and Syncthing never see two writers on one file.
2. Merge: the union of all devices' operations, ordered by Lamport clock and device id. Annotation properties are
   last-writer-wins registers; deletes are tombstones; note text uses a text CRDT (Automerge or yrs).
3. The PDF is rebuilt from the base revision plus merged operations; "conflicted copy" files from the sync service are
   ignored and cleaned up.
4. Optional end-to-end encryption of the log files (`age`).

**Done when:** two devices edit offline, sync, and end with identical annotations and notes (property test with
random interleavings).
**Links:** Automerge <https://automerge.org>; yrs <https://github.com/y-crdt/y-crdt>; age
<https://github.com/str4d/rage>

#### Ph9: 10/13 | Y.2: Same settings on every OS
**P1; Phase 9; Needs:** K.16, Y.1; **Code:** `excellent-app::settings::sync`
1. Settings, keymap, pens, palettes, stamps and templates in the sync folder; per-OS overrides in their own table.

**Done when:** a pen added on Windows appears on macOS.

#### Ph9: 11/13 | Y.3: Share bundles
**P2; Phase 9; Needs:** N.4; **Code:** `excellent-features/share`
1. `.exbundle`: a zip of the PDF with annotations, notes, audio and boards; import merges by ids (K.14).

**Done when:** a bundle round-trips between two users without duplicates.

#### Ph9: 12/13 | Y.4: Live co-editing through a relay
**P2; Phase 9; Needs:** Y.1; **Code:** `crates/excellent-relay`
1. A small self-hostable relay (WebSocket) that forwards encrypted operation-log messages; invite links carry the
   key; presence and cursors.
2. Publish the relay's source, as AGPL §13 requires for network use (§4).

**Done when:** two users annotate the same page live with under 300 ms delay.

### 27.16 Extensibility and automation (E)

#### Ph5: 41/45 | E.1: Command-line batch tools
**P1; Phase 5; Needs:** K.3; **Code:** `crates/excellent-cli`
1. Subcommands on the same core: `merge`, `split`, `convert`, `ocr`, `redact`, `compress`, `extract-text`, `render`,
   `info`, with `clap`.
2. JSON output option for scripts.

**Done when:** each subcommand has an integration test on the corpus.
**Links:** clap <https://github.com/clap-rs/clap>

#### Ph5: 42/45 | E.2: Agent tool table and `excellent-cli run`
**P1; Phase 5; Needs:** E.1; **Code:** `excellent-core::tools` (Appendix C.3.1)
1. One table of tools, each with a JSON Schema for input and output (`schemars`): open, render, find text, list and
   add annotations, fill forms, organize pages, save.
2. `excellent-cli run tool.json` runs one call or a script of calls.

**Done when:** every tool has a schema test and an example.
**Links:** JSON Schema <https://json-schema.org>; schemars <https://github.com/GREsau/schemars>

#### Ph5: 43/45 | E.3: MCP server
**P1; Phase 5; Needs:** E.2; **Code:** `excellent-cli mcp`
1. Serve the tool table over stdio as an MCP server, opt-in, confined to a `--root` folder (paths checked against
   it).

**Done when:** an MCP client can open a file under the root and can't open one outside it.
**Links:** Model Context Protocol <https://modelcontextprotocol.io>

#### Ph5: 44/45 | E.4: UI control channel
**P1; Phase 5; Needs:** E.2; **Code:** `excellent-app::control`
1. Opt-in loopback channel; a random token in a file only the user can read; commands from the command table plus
   state queries and screenshots (like Enhanced's `-dbg-control`).
2. Used by UI tests and agents.

**Done when:** the UI test suite drives the app through it.

#### Ph9: 13/13 | E.5: Plugins
**P2; Phase 9; Needs:** E.2, K.21; **Code:** `crates/excellent-plugins`
1. Signed TSX packages running in NSH's sandbox with capabilities the user grants (read document, add annotations,
   network); they call the tool table.
2. Plugin manager (install, update, disable) using the pack manager's signing.

**Done when:** a sample plugin works, and one without the right capability is refused.
**Links:** NSH sandbox `C:\dev\nsh-src\website\docs\sandbox-and-capabilities.md`, signed packages
`signed-packages.md`

### 27.17 AI (I)

All AI follows §12.3. Document text is untrusted data: it goes into prompts as quoted material, and models that read
it get no tools that act without the user's confirmation.

#### Ph6: 4/13 | I.1: Providers
**P1; Phase 6; Needs:** K.21, K.19; **Code:** `crates/excellent-ai::providers`
1. A `Provider` trait (chat, streaming, embeddings). Local: llama.cpp (pack) or a running Ollama; cloud: any provider
   the user configures (OpenAI-compatible endpoints and vendor APIs), keys in the OS key store.
2. Consent per provider and per workspace; a visible local or cloud badge; off switch per document and workspace.
3. Port the provider list and settings from Enhanced's `src/AIChat*.cpp`.

**Done when:** the same feature works with a local model and a cloud one; with AI off, no AI code runs.
**Links:** llama.cpp <https://github.com/ggml-org/llama.cpp>; Ollama API
<https://github.com/ollama/ollama/blob/main/docs/api.md>

#### Ph6: 5/13 | I.2: Document index for AI
**P1; Phase 6; Needs:** F.3, I.1; **Code:** `crates/excellent-ai::index`
1. Chunk K.13 text into paragraph windows of about 512 tokens with overlap; keep page and quads (§12.2).
2. Embeddings with a local ONNX model pack (`ort`); vectors in `usearch`; rebuildable from text.
3. Hybrid retrieval: FTS5 score plus vector similarity plus metadata filters.

**Done when:** retrieval recall@10 is measured on a question set and tracked in CI.
**Links:** ort <https://github.com/pykeio/ort>; usearch <https://github.com/unum-cloud/usearch>

#### Ph6: 6/13 | I.3: Ask with cited answers
**P1 (parity: AI chat panel); Phase 6; Needs:** I.2; **Code:** `excellent-features/ai/chat.rs`
1. Chat panel (NSH `TextView` for Markdown answers) scoped to selection, page, document, workspace or library.
2. The model must cite chunk ids; every quote is checked to appear verbatim (after normalization) in its chunk;
   unverified claims are marked, never shown as cited.
3. Clicking `[1]` scrolls the canvas to the quads and pulses a highlight.

**Done when:** citation accuracy on the evaluation set (I.12) meets its target.

#### Ph6: 7/13 | I.4: Summaries; I.5: Plain-language rewrite and explanation; I.7: Define terms in context
**P1; Phase 6; Needs:** I.3; **Code:** `excellent-features/ai/actions.rs`
1. Selection-toolbar and palette actions with fixed prompt templates; output in the chat panel with citations;
   "insert as note" only on request.

**Done when:** each action has prompt tests with expected properties (length, citations present).

#### Ph6: 8/13 | I.6: Explain equations step by step
**P1; Phase 6; Needs:** I.3; **Code:** as I.4
1. Take the equation as LaTeX (from the text layer, or H.25 / image-to-LaTeX for images); render steps with KaTeX in
   the panel.

**Done when:** equations from the corpus papers are recognized and explained with their page cited.

#### Ph6: 9/13 | I.8: Glossary, study guide and flashcard generation
**P1; Phase 6; Needs:** I.3, S.1; **Code:** `excellent-features/ai/study.rs`
1. Generate drafts; the user reviews and accepts each item before it becomes a card or note (§12.3).

**Done when:** nothing is added without acceptance.

#### Ph6: 10/13 | I.9: Claim, method, finding and limitation extraction
**P1; Phase 6; Needs:** I.3, A.17; **Code:** `excellent-features/ai/extract.rs`
1. Structured output (JSON Schema) with a quote and page for each item; offered as suggested annotations with semantic
   categories (A.17).

**Done when:** every extracted item has a verified quote.

#### Ph6: 11/13 | I.10: Cross-document comparison and evidence tables
**P1; Phase 6; Needs:** I.9; **Code:** as I.9
1. Run I.9 over chosen documents; build a table (rows: documents; columns: method, sample, result, limitation) with
   citations in each cell; export as CSV or Markdown.

**Done when:** each cell links to its source passage.

#### Ph6: 12/13 | I.11: Suggested highlights and tags
**P2; Phase 6; Needs:** I.9; **Code:** as I.9
1. Suggestions shown as ghost highlights; accept or dismiss each; never applied silently.

**Done when:** dismissing leaves the document unchanged.

#### Ph6: 13/13 | I.12: Evaluation and safety suite
**P1; Phase 6; Needs:** I.3; **Code:** `tests/ai-eval/`
1. Question sets with known answers and pages; adversarial documents with injected instructions; measure citation
   accuracy, hallucination rate and injection resistance per provider.

**Done when:** the suite runs on demand and before each release, with results in the release notes.

### 27.18 Universal access: accessibility, read aloud and languages (U)

#### Ph1: 56/60 | U.1: Keyboard access and focus
**P0; Phase 1; Needs:** K.16; **Code:** everywhere
1. Logical focus order and visible focus in every view; every command reachable by keyboard; a keyboard test per
   feature (`nsh test`).

**Done when:** the core tasks of each mode can be done without a mouse.

*Built (2026-11-15; `docs/features/U.1-keyboard-access.md`):* every command in the palette, visible focus rings,
dialogs that keep and give back the focus, a headless test that reads a document from the keyboard alone. Read
mode only: the other modes come with K.18.

#### Ph1: 57/60 | U.2: Canvas accessibility on Windows
**P0; Phase 1; Needs:** K.8, K.13; **Code:** `crates/excellent-canvas-win::uia`
1. Answer `WM_GETOBJECT` with a UI Automation provider (`UiaReturnRawElementProvider`): fragments for pages, links,
   annotations and form fields.
2. Text pattern (`ITextProvider`, `ITextRangeProvider`) over K.13 text in reading order, so Narrator and NVDA can read
   and select document text; tagged PDFs use their structure and alt text.

**Done when:** NVDA reads a page in order, follows links and reads annotation notes.

*Built (2026-11-15; `docs/features/U.2-canvas-accessibility.md`):* a UI Automation provider on the canvas window
(`excellent-canvas-win::uia`): a Document with the Text pattern over K.13 text (`excellent-text::walk`), pages,
links (Invoke) and annotation notes as children. Pages far from the screen read as empty until their text is
read; tagged-PDF structure and form fields come later. Not tried with NVDA or Narrator.
**Links:** UI Automation text pattern
<https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-implementingtextandtextrange>; NSH accessibility
`C:\dev\nsh-src\website\docs\accessibility.md`

#### Ph2: 3/3 | U.3: Canvas accessibility on macOS, Linux and tablets
**P0; Phase 2 (desktop), Phase 8 (tablets); Needs:** U.2; **Code:** canvas crates
1. macOS NSAccessibility (text area with ranges); Linux AT-SPI; iPad UIAccessibility; Android accessibility nodes.
2. Evaluate AccessKit for the desktop providers before writing three by hand.

**Done when:** VoiceOver and Orca read pages in order.
**Links:** AccessKit <https://github.com/AccessKit/accesskit>

#### Ph1: 58/60 | U.4: High contrast, color-blind-safe colors, scaling and reduced motion
**P0; Phase 1; Needs:** K.17; **Code:** `excellent-app::theme`
1. Follow the OS high-contrast, scaling and reduced-motion settings; color-blind-safe annotation palettes; never
   use color alone to carry meaning.

**Done when:** every screen passes the contrast check at 200 % scaling.

*Built (2026-11-15; `docs/features/U.4-high-contrast-scaling-motion.md`):* Windows' high contrast turns on each
theme's high-contrast side (setting `follow_system_contrast`, default on); text scaling and reduced motion are
NSH's. Not looked at by hand at 200 %.

#### Ph4: 44/44 | U.5: Read aloud
**P0 (parity); Phase 4; Needs:** K.13, K.19; **Code:** `excellent-features/read-aloud`
1. Port Enhanced's read aloud (`src/ReadAloud.cpp`): speak from the cursor in reading order; sentence and word
   highlighting from TTS word-boundary events mapped to K.13 quads.
2. TTS per platform (OneCore / SAPI, AVSpeechSynthesizer, speech-dispatcher, Android TTS) behind K.19; the `tts`
   crate covers several.
3. Pause, speed, voice; continue across pages.

**Done when:** highlighting stays on the spoken word for a full chapter.
**Links:** tts-rs <https://github.com/ndarilek/tts-rs>

#### Ph5: 45/45 | U.6: Neural offline voices
**P1; Phase 5; Needs:** U.5, K.21; **Code:** `packs/voices/`
1. Piper-style ONNX voices as packs, run with `ort`; check each voice's licence.

**Done when:** a pack voice reads with word highlighting, offline.
**Links:** Piper <https://github.com/rhasspy/piper>

#### Ph1: 59/60 | U.7: Localization
**P0 infrastructure, P1 languages; Phase 1; Needs:** K.1; **Code:** `app/locales/*`, `excellent-app::l10n` (Appendix D.4)
1. NSH's translations (`nsh-i18n`, which reads the app's translations with rust-i18n, v0.5.0), so our strings and
   NSH's own components share one locale and one set of number and date formats. English is the source; a
   pseudo-locale test catches hard-coded text. If rust-i18n's plural handling is too weak for a language, keep
   Fluent (`fluent-bundle`) for those messages only.
2. Right-to-left UI mirroring through NSH's layout direction.
3. Vertical CJK text in ebooks: check MuPDF's HTML engine for `writing-mode: vertical-rl`; record a gap if missing.

**Done when:** the pseudo-locale test passes; the UI works in Arabic.

*Built (2026-11-15; `docs/features/U.7-localization.md`):* every string in `app/locales/en.json` and `ar.json`,
a language setting, right-to-left mirroring, and the pseudo-locale test. The Arabic is not reviewed by a native
speaker; vertical CJK text waits for the ebook work.
**Links:** NSH i18n `C:\dev\nsh-src\crates\nsh-i18n`; rust-i18n
<https://github.com/longbridge/rust-i18n>; Fluent (fallback) <https://projectfluent.org>

#### Ph3: 49/49 | U.8: Input methods on the canvas
**P0; Phase 3; Needs:** K.8; **Code:** canvas crates (Appendix D.1.4)
1. Windows: IMM32 composition messages first (`WM_IME_STARTCOMPOSITION`, `WM_IME_COMPOSITION`, candidate window placed
   at the caret); move to TSF if needed.
2. macOS `NSTextInputClient`; GTK `GtkIMContext`; iPad `UITextInput`; Android `InputConnection`.
3. All in-canvas text editing (A.6, N.6, O.5, O.11) uses this path.

**Done when:** Chinese, Japanese and Korean input works in text boxes and form fields on every desktop.

*Status (2026-11-25):* not built yet (DD26). The canvas takes no text; text boxes and every other field are NSH's,
which compose with each desktop's input method. The canvas path comes with N.6, the first in-canvas editor.
**Links:** IME on Windows <https://learn.microsoft.com/en-us/windows/win32/intl/input-method-manager>;
NSTextInputClient <https://developer.apple.com/documentation/appkit/nstextinputclient>

#### Ph1: 60/60 | U.9: First-run guide and tips
**P1; Phase 1; Needs:** K.18; **Code:** `excellent-features/onboarding`
1. A short skippable guide (open, modes, palette); contextual tips shown once; "replay guide" in Help.

**Done when:** the guide never blocks opening a file passed on the command line.

*Built (2026-11-15; `docs/features/U.9-first-run-guide.md`):* a three-step guide (open, reading options, palette)
at the first start without files, two tips shown once, "Show the getting-started guide" in the palette and in
Settings, Getting started (no Help menu yet). K.18 is not built, so the second step teaches the reading options
instead of the modes (DD7).

### 27.19 Tags by phase

Use this table to pick the next work. Within a phase, build **Needs** first.

| Phase | Tags |
| --- | --- |
| 0 | K.1, K.2, K.3, K.4, K.7 (start), K.8 (spike), K.11 (spike), K.22 (spike), K.23, K.24, K.26, K.27, H.1 (spike) |
| 1 | K.5, K.6, K.7, K.8, K.13, K.14 (design), K.15 (read-only), K.16, K.17, K.18, K.19, K.20, K.22, K.25, R.1–R.15, R.19–R.21, R.28, B.1–B.6, V.1–V.7, V.13, W.1–W.7, W.14, F.1, F.17, O.22 (Windows), U.1, U.2, U.4, U.7, U.9 |
| 2 | K.9, K.10, O.22 (macOS, Linux), U.3 (desktop) |
| 3 | K.14, K.15 (edits), K.28, R.17, R.25, A.1–A.22, A.24, A.25, A.28, B.7, B.9, B.10, V.8–V.11, W.8, W.9, W.11, H.1–H.3, H.5–H.10, O.23, O.25, U.8 |
| 4 | K.21 (start), R.23, N.1–N.10, N.14, L.1, L.3, S.1–S.8, S.10–S.12, G.1–G.8, H.4, H.11–H.18, H.22, H.26, U.5 |
| 5 | K.21 (finish), R.16, R.22, R.24, R.27, R.29, R.30, B.8, B.11, B.13, V.12, W.10, W.12, N.11–N.13, L.2, S.13–S.16, F.2–F.13, O.4, A.26, A.27, H.19–H.21, H.23, H.24, E.1–E.4, U.6 |
| 6 | R.31, S.9, H.25, I.1–I.12 |
| 7 | R.18, R.26, B.12, A.23, G.9–G.13, F.15, F.16, L.4, O.1–O.3, O.5–O.20, O.24 |
| 8 | K.11, K.12, V.14, O.21, U.3 (tablets) |
| 9 | W.13, X.1–X.4, Y.1–Y.4, G.14, G.15, F.14, E.5 |
