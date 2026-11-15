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

