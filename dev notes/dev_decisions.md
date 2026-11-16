# Development decisions

Decisions the maintainer made while building, with what they mean in the code. The plan's fixed decisions (D1–D10)
and ADRs stay where they are; this file records the working decisions taken since, newest last. When one of these
changes the plan, the plan is updated in the same change.

## DD1. All of MuPDF, on every operating system (2026-11-13)

**Decision (maintainer):** ExcellentPDF is cross-platform: Windows, Linux, macOS, Android, iOS / iPadOS, and any
other OS Rust and a C compiler can reach. Keep everything inside MuPDF; build none of it out.

**What it means:**

- `excellent-mupdf-sys/build.rs` compiles every part of MuPDF that upstream SumatraPDF compiles, on every target.
  This replaces the Phase 0 build, which left out OCR (Tesseract, Leptonica), barcodes (zxing-cpp, zint), archives
  (libarchive with bzip2 and liblzma), SumatraPDF's OFD reader and MuPDF's `tools` directory.
- The build follows upstream exactly: `premake5.lua` / `premake5.files.lua` for Windows, and
  `cmd/helper/ng-targets.ts` for Linux and macOS. Upstream does not build Android or iOS; those follow the nearest
  upstream platform (Linux for Android, macOS for iOS), as the table below shows.
- `shim.c` registers SumatraPDF's OFD handler next to MuPDF's own document handlers.

| Part | Windows | Linux | macOS | iOS / iPadOS | Android |
| --- | --- | --- | --- | --- | --- |
| JPEG XR | `load-jxr-win.c` (WIC) | `load-jxr.c` (not supported) | `load-jxr.c` | `load-jxr.c` | `load-jxr.c` |
| Signatures | `pkcs7-windows.c` | `pkcs7-openssl.c` (OpenSSL) | `pkcs7_mac.c` (Security) | `pkcs7_stub_posix.c` (none) | `pkcs7_stub_posix.c` (none) |
| System fonts | Windows font folder | fontconfig | Core Text | Core Text | none yet (built-in fonts only) |
| libarchive config | `config_windows.h` | `config_posix.h` | `config_posix.h` | `config_posix.h` | `config_posix.h` (iconv from API 28) |
| System libraries | ole32, windowscodecs, user32, gdi32, advapi32, shell32, crypt32, ncrypt, bcrypt | fontconfig, crypto | iconv; CoreFoundation, CoreGraphics, CoreText, Security | as macOS | none |

- On x86 targets libjpeg-turbo's SIMD code is assembled with NASM (found on `PATH` or through the `NASM`
  variable). Without NASM the build still succeeds, without SIMD, and prints a warning. CI installs NASM, so release
  builds always have SIMD. ARM targets have no assembly.
- Tesseract's x86 kernels (AVX, AVX2, FMA, SSE4.1) are compiled with their own flags and chosen at run time, as
  upstream does; on ARM they are left out.
- Not built, because they are programs, not library code: `cmapdump.c`, and the `main` files `mutool.c` and
  `muraster.c`. MuPDF's `noto.c` is replaced by SumatraPDF's `noto_sumatra.c`, as upstream does.

**Gaps to close in their phases** (no stubs hidden; each is visible in the table above):

- Android: no system font fallback (`mupdf_load_system_font.c` is built with `-U__linux__`, so it skips
  fontconfig); no signature checking; iconv only from API 28, so `cargo ndk --platform 28` in CI.
- iOS and Android: no signature checking (`pkcs7_stub_posix.c`).
- Android and iOS builds are untested on devices until Phase 8 (tablets).

**Tension with D8 (flagged, not decided):** D8 says optional features are packs, never in the core installer, and the
plan made OCR a pack. Building OCR and barcodes into the core library adds several MB of code against the 15 MB core
installer budget. Tesseract's language data (`traineddata`) stays a pack either way. If the size budget fails, the
choice is the maintainer's: keep everything in, or move OCR back into a pack (plan §6, §21).

## DD2. GitHub Actions for every platform, Windows on every change (2026-11-13)

**Decision (maintainer):** GitHub builds every platform automatically, and the infrastructure is always in place, but
for now only Windows is built.

**Update (maintainer, 2026-11-15):** from Phase 2 on, every supported desktop (Windows, macOS, Linux) is built and
tested on every push, and each build uploads the app as an artifact (the Windows MSI; the release binary, archived,
on macOS and Linux, until their packaging exists). Platforms this machine can't build are checked through these runs
and their artifacts. Test builds carry N = 0 in their names (DD3). Tablets stay by hand until Phase 8.

**What it means:**

- `.github/workflows/build.yml` is one reusable workflow (`workflow_call`) that builds, lints and tests one Rust
  target on one runner: installs the toolchain and target, NASM on x86, fontconfig and OpenSSL headers on Linux,
  `cargo-ndk` on Android.
- One thin workflow per platform calls it:

  | Workflow | Targets | Runs |
  | --- | --- | --- |
  | `windows.yml` | `x86_64-pc-windows-msvc` (tests, clippy); `aarch64-pc-windows-msvc` | every push to `main` and every pull request; arm64 by hand. Also `cargo fmt` and `cargo deny` |
  | `linux.yml` | `x86_64-` and `aarch64-unknown-linux-gnu` (tests) | every push to `main` and every pull request since Phase 2 |
  | `macos.yml` | `aarch64-` and `x86_64-apple-darwin` (tests) | every push to `main` and every pull request since Phase 2 |
  | `android.yml` | `aarch64-` and `x86_64-linux-android` (build only) | by hand until Phase 8 |
  | `ios.yml` | `aarch64-apple-ios`, `aarch64-apple-ios-sim` (build only) | by hand until Phase 8 |

- "By hand" means `workflow_dispatch`: run it from the Actions tab whenever a change might break that platform. To
  turn a platform on for every change, copy the `push` and `pull_request` triggers from `windows.yml`.
- `fuzz.yml` (T4 and T5, plan K.3 and K.4): the corpus and render stress tests under AddressSanitizer with leak
  checks, then five minutes of the `open_render` fuzz target (`tests/fuzz/`). Linux only, since both tools need it;
  weekly (Monday 03:00 UTC) and by hand. K.23 makes it nightly and adds targets.
- Still to come from plan §17: `budgets` and `release` workflows, with the tasks that need them (T10, K.24).

## DD3. Release names (2026-11-14)

**Decision (maintainer):** every release of the app is named `App-vYY-MM-DD.N-vSEMVER-platform`.

**What it means:**

- `App` is the app's name, `ExcellentPDF`.
- `YY-MM-DD` is the release date, two digits each.
- `N` counts releases since the first one was published: the first release is `.1`, and it never resets (not per day,
  not per version).
- `SEMVER` is the workspace version from `Cargo.toml` (`0.1.0`).
- `platform` is the build's platform, such as `windows-x64` or `windows-arm64`.
- Example: the third release ever, version 0.4.2 for 64-bit Windows on 9 March 2027, is
  `ExcellentPDF-v27-03-09.3-v0.4.2-windows-x64`.
- Git tags, release titles and installer file names use it. The `release` workflow (plan §17) builds it when it is
  written.

## DD4. Pane keys: Alt+1 to Alt+4 and F6 (2026-11-15; proposed, waiting for the maintainer)

**Proposal (agent, not yet approved):** V.5's "Ctrl+1…Ctrl+4 focus panes" clashes with R.12, where Ctrl+1 to Ctrl+9
go to tabs as in SumatraPDF Enhanced and the browsers. Panes take **Alt+1 to Alt+4** instead, with **F6** and
**Shift+F6** for the next and previous pane (the common Windows key for moving between a window's panes).

**What it means:**

- Built this way in the V group; the plan's V.5 entry carries a note pointing here.
- Every key can be changed in `settings.toml` (K.16), so a reader who prefers Ctrl+1 to 4 for panes can swap them.
- If the maintainer prefers the plan's keys, the tabs' Ctrl+1 to 4 move instead; one line each in the command table.

## DD5. Find matches in Rust, not with `fz_search_stext_page_cb` (2026-11-15; agent's call, flagged)

**Decision (agent):** F.1 says to search "using `fz_search_stext_page_cb` on K.13 text". MuPDF's search folds case
only; it has no whole-word option and does not fold accents, which F.1 asks for. F.1 reads each page's text with
K.13's `TextLayout` (the same call, on the render workers) and matches in Rust (`excellent-features::search::find`),
so the options are ours and the hits' quads come from K.13's `PageText::highlight`, as a selection's do.

**What it means:**

- No new MuPDF call or shim; matching is pure Rust and unit-tested.
- If the maintainer prefers MuPDF's search, only `find_on_page` changes; whole words and accents then go.

## DD6. Print scaling and "print annotations" are settings, not dialog controls (2026-11-15; agent's call, flagged)

**Decision (agent):** O.22 asks for a print dialog with range, copies, scaling, duplex, colour, "print annotations"
and "print as image". Windows' own dialog (`PrintDlgExW`) has range, copies, duplex and colour; adding scaling and
annotations to it needs a custom property page (a Win32 dialog template), as SumatraPDF has. O.22 keeps the system
dialog as it is and puts **Page size on paper** (shrink, fit, actual) and **Print annotations** in the settings
window's Printing group. Every page prints as an image at the printer's resolution, so "print as image" is not a
choice.

**What it means:**

- One dialog the reader knows from every Windows app; the two extra choices are kept between prints.
- If the maintainer wants them in the dialog, a property page can be added in `excellent-platform::print` alone; the
  settings stay as the defaults it opens with.

## DD7. U.9 before K.18: the guide teaches the reading options, not the modes (2026-11-15; agent's call, flagged)

**Decision (agent):** U.9 needs K.18 (modes and toolbars), which isn't built. Rather than stop the last Phase 1 entry,
U.9 ships its guide with the steps that exist: open a document, the reading options (in Settings), and the palette.
The plan's "replay guide in Help" is the palette command **Show the getting-started guide** and a button in
Settings, Getting started, because there is no Help menu or toolbar yet.

**What it means:**

- When K.18 lands, the second step names the modes and their toolbars, and the guide gets a toolbar or menu entry.
  `excellent-features::onboarding::Step` changes in one place; the settings keys stay.
  (2026-11-15: K.18 landed; the step now names the modes. The guide is still reached from the palette and Settings.)
- If the maintainer prefers U.9 to wait for K.18, the guide can stay off (the desktop binary's
  `offer_first_run_guide` call) without removing anything.

## DD8. K.18's modes before their tools (2026-11-15; agent's call, flagged)

**Decision (agent):** K.18 ships the five modes of §10.2 while most of their tools are later tags. So that the bar
never offers an empty mode:

- Write and Study are left out of the switcher until their toolbar has a tool (the H.* and S.* tags, or the reader's
  own customizing) or one of them is the current mode. Pen contact switches to Write only once Write is offered.
- Annotate's default tools are the ones that exist: add a bookmark, add an outline entry, undo, redo.
- Tools are text buttons with short labels (the full label and shortcut are their accessible names): there is no icon
  set yet. A narrow window scrolls the tools sideways.
- Next and previous mode are Ctrl+Shift+M and Ctrl+Alt+Shift+M (Ctrl+M minimizes on macOS).
- The mode is one app-wide setting, not per workspace or per tab.
- The pen still draws ink in every mode; H.1 decides what a pen does outside Write.

**What it means:**

- When a tag adds tools, it adds its command names to `excellent-app::modes::default_words`; the mode then appears.
- Icons replace the short labels in one function (`tool_button`) when an icon set is chosen.
- If the maintainer wants the mode per workspace, `Modes` reads and writes it through the workspace instead of
  `settings.toml`; the toolbars stay app-wide.


## DD9. One page canvas for every platform, behind a `Host` trait (2026-11-15; agent's call, flagged)

**Decision (agent):** K.9 and K.10 item 3 say the shared logic stays in `excellent-canvas`. Before them, all of it
lived in `excellent-canvas-win`, tied to Win32 and Direct2D. It now lives in `excellent_canvas::page`:

- `PageCanvas<H: Host>` holds the document, layout, viewport, tiles, input, selection, links, find, the reading aids
  and ink, and parses the component's JSON props and commands (`set_prop`, `command`). Each platform implements
  `page::Host` for its view: size, redraws, events, waking from workers, uploading tiles and a few drawing calls
  (fill, image, highlight, band, dim outside, polyline), with optional hooks for screen readers and an ink trail.
- The reader's view model (`view`, `view/history`) moved from `excellent-core` to `excellent-canvas`, since only the
  canvas uses it; `excellent-core` no longer depends on `excellent-canvas`, which keeps the crates acyclic.
- `formats::reflow_pages` takes the room inside the layout's gaps; the canvas subtracts them.
- `excellent-canvas-win` keeps the window, Direct2D (`gfx`), `WM_POINTER` input, UI Automation and the `label` prop.
  Pen samples reach the shared canvas in the view's pixels (`PenSample`).

**What it means:**

- The GTK and macOS canvases are a host, input wiring and the `label` prop each; behaviour fixes land once.
- A host that can draw only inside its toolkit's draw callback (GTK) says so through `Host::frame_ready`.
- If the maintainer prefers each platform to keep its own canvas logic, this is reverted by moving `page` back; no
  public behaviour changed.

## DD10. The macOS canvas draws with Core Graphics, not a `CAMetalLayer` (2026-11-15; agent's call, flagged)

**Decision (agent):** K.9 item 1 says an `NSView` backed by a `CAMetalLayer`, with tiles as Metal textures. The first
macOS canvas instead draws in the view's `drawRect:` with Core Graphics: tiles are `CGImage`s, highlights blend with
`kCGBlendModeDarken`, the reading aids fill even-odd paths, and ink strokes with round caps. The view is layer-backed,
so AppKit composites it on the GPU.

**Why:** every drawing call the shared canvas makes (`page::Host`) maps to one Core Graphics call, so the macOS host is
one file with no shaders, pipelines or texture management, and it can be checked only through CI on this machine.
The item's own third line ("draw overlays with Metal or Core Graphics") already allows Core Graphics for overlays.

**What it means:**

- Scrolling redraws the visible tiles each frame on the CPU-side Core Graphics path; K.24's budgets on a Mac decide
  whether that holds 60 fps (120 on ProMotion screens).
- If it doesn't, a `CAMetalLayer` host replaces `host.rs` alone: the view, input and component stay.
- If the maintainer wants Metal from the start, this is the file to replace.

## DD11. The margin column sits beside the page canvas, not over it (2026-11-16; agent's call, flagged)

**Decision (agent):** A.11 item 1 says "a margin column beside the page (canvas overlay)". The column is an NSH view
in the pane's row, to the right of the page canvas, rather than something the canvas draws over the pages.

**Why:** the canvas is a native window of its own (D5; on Windows a child window), and NSH can't lay its views over
it. The canvas has no text drawing (`page::Host` draws fills, images and lines), and the cards need wrapped text, the
platform's text input for writing in place, focus and screen-reader names, all of which NSH has. The canvas instead
reports each page in view as a map from its points to the app's pixels (`margin` events, every frame the pages
move), and the column places the cards with `excellent-features`' `annotate::margin::layout`.

**What it means:**

- The pages get narrower by the column's width (280 px) while it shows; fit-width zoom follows.
- Cards are level with their notes but in their own column; there are no leader lines across the page.
- Item 3 ("Expand margins", N.3) is Phase 4's and not built yet; when it is, the column can sit in the widened margin.
- If the maintainer wants the cards drawn on the canvas itself, the canvas needs text drawing in all three hosts, and
  an input path for writing in place (U.8); the layout model stays.

## DD12. The annotation clipboard is the app's own, without the image fallback (2026-11-16; agent's call, flagged)

**Decision (agent):** A.13 item 2 asks for "internal clipboard format plus an image and text fallback". Copied
annotations are kept whole inside the running app (`excellent-features`' `annotate::edit::Clip`, serialized as
`excellentpdf.annotations/1`), shared by its documents, panes and windows; their notes go on the system clipboard as
text. No image is put on the clipboard, and the format is not registered with the system.

**Why:** NSH 0.5.0's clipboard takes text and images only (`set_clipboard`, `set_clipboard_image`); a format of the
app's own needs a registered clipboard format in each host. An image needs the annotations rendered alone, off the
UI thread, which is a render job of its own.

**What it means:**

- Pasting between two running copies of the app, or into another app as a picture, doesn't work yet.
- When NSH or the hosts take custom formats, `Clip::to_json` is the payload and paste reads it back with `from_json`.

## DD13. The annotation list is a tab beside the inspector, with a colour bar for its rows (2026-11-16; agent's call, flagged)

**Decision (agent):** A.14 item 1 asks for a sidebar tab whose rows show a type icon. The list is a tab of the right
dock, next to the inspector and the find results, and each row starts with a bar in the annotation's colour, its
kind's name under its text.

**Why:** the left sidebar is 260 px wide and holds three tabs already; a fourth doesn't fit its tab strip. In the
right dock, a row pressed in the list opens its note in the inspector one tab over. The app has no icon set for the
annotation kinds yet, and the kind named in words is clearer than a small glyph.

**What it means:**

- The tab can be dragged to the left sidebar like any dock panel.
- When the annotation kinds get icons, the row shows the icon in the bar's place.

## DD14. The transfer review is a dialog of counts, not a list (2026-11-16; agent's call, flagged)

**Decision (agent):** A.16 item 1 asks for "a review list (placed, moved, orphaned) before applying". The review is
a dialog: how many were found where they were, found elsewhere or changed, kept in place and not found, the first
five not found named, and Add or Cancel. It adds every annotation that was found; none can be left out one by one.

**Why:** a list to pick from needs a dialog of its own with a scrolling list of rows, and the annotation list (A.14)
already shows every annotation once added, where any can be checked, deleted (A.13) or undone in one step.

**What it means:**

- Moved annotations aren't listed before they are added; the reader checks them in the Annotations tab afterwards.
- A review with rows and a check box each can replace the dialog without changing `annotate::transfer`.

## DD15. Categories are picked in the inspector, drawn as colour dots (2026-11-16; agent's call, flagged)

**Decision (agent):** A.17 item 2 says "pick from the selection toolbar". The category is picked in the annotation's
inspector, as a row of seven chips, each a dot in its colour and its name, under the note. Each category names its
icon (Lucide: megaphone, file-check, circle-help, book-open, flask-conical, chart-column, square-check), but the chips
draw the dot, not the icon.

**Why:** the canvas has no selection toolbar of annotation actions yet; the inspector already opens on every selected
annotation and holds its other properties (colour, author, lock). The app ships no icon set; drawing seven icons needs
one, which belongs to the design system, not to this tag.

**What it means:**

- Choosing a category takes a click on the annotation, then one on the chip; no shortcut sets one.
- When a selection toolbar or an icon set lands, both read `Category::ALL`, `color` and `icon`: no model change.
- A locked annotation's category can't be changed, as its colour can't (A.12).
