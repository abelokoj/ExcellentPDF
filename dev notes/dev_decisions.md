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

**What it means:**

- `.github/workflows/build.yml` is one reusable workflow (`workflow_call`) that builds, lints and tests one Rust
  target on one runner: installs the toolchain and target, NASM on x86, fontconfig and OpenSSL headers on Linux,
  `cargo-ndk` on Android.
- One thin workflow per platform calls it:

  | Workflow | Targets | Runs |
  | --- | --- | --- |
  | `windows.yml` | `x86_64-pc-windows-msvc` (tests, clippy); `aarch64-pc-windows-msvc` | every push to `main` and every pull request; arm64 by hand. Also `cargo fmt` and `cargo deny` |
  | `linux.yml` | `x86_64-` and `aarch64-unknown-linux-gnu` (tests) | by hand until Phase 2 |
  | `macos.yml` | `aarch64-` and `x86_64-apple-darwin` (tests) | by hand until Phase 2 |
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
