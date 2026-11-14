# Research notes

Findings that inform the plan (`ExcellentPDF_detailed_plan_nov_9th.md`). Newest first. Each entry says when it was
checked and how to check it again.

---

## R1. The MuPDF inside SumatraPDF Enhanced (checked 2026-11-12)

**Short answer:** Enhanced's MuPDF is upstream MuPDF **1.28.5** plus **36 patches**. It is **not newer** than
upstream: MuPDF has since released **1.28.6**, and MuPDF's `master` is already **1.29.0** (unreleased). But the
patches add things upstream MuPDF does not have (WebP, JPEG XL and JPEG XR images, Windows signing, many EPUB, HTML,
SVG and annotation fixes). So it is behind in version, ahead in features. Upstream SumatraPDF has meanwhile added
40 more patches on the same 1.28.5 base that Enhanced does not have.

### R1.1 What is vendored

| Item | Value |
| --- | --- |
| Tree | `C:\dev\sumatrapdf\ext\mupdf` (1,410 files: a subset of MuPDF, not the whole repository) |
| Version | `FZ_VERSION "1.28.5"` (`include/mupdf/fitz/version.h`); base commit `8ad45e92f`, released 2026-09-24 (`ext/versions.txt`) |
| Changes | 36 patches in `ext/patches/`, each with a description in `ext/patches/README.md` |
| Claim in that README | `ext/mupdf` is byte-for-byte 1.28.5 plus these patches and nothing else |
| Our check | All 36 patches reverse-apply cleanly to `ext/mupdf` (`git apply -R --check`), so every one is present. The byte-for-byte comparison against a pristine 1.28.5 was not re-run (it needs the upstream 1.28.5 source; the README gives the commands) |
| Code outside the tree but compiled into the `mupdf` project | `src/mupdf/`: `pkcs7-windows.[ch]` (signing through Windows CryptoAPI instead of OpenSSL), `load-jxl.[cpp,h]` (JPEG XL), `mupdf_load_system_font.c`, `noto_sumatra.[ch]` |
| Third-party libraries | Not MuPDF's own `thirdparty/` submodules: SumatraPDF builds FreeType, HarfBuzz, lcms2, brotli and others from its own amalgamations in `ext/a-*` (versions in `ext/versions.txt`) |

### R1.2 Compared with upstream MuPDF

| Upstream | State on 2026-11-12 |
| --- | --- |
| 1.28.5 | What Enhanced uses |
| **1.28.6** (latest release, commit `da5f5eb89`, 2026-10-29) | Only 4 commits after 1.28.5: version strings and an update of MuPDF's bundled **brotli** to just after v1.2.1. SumatraPDF does not use MuPDF's bundled brotli; it builds `ext/a-brotli`, which is **brotli 1.2.0**. So the useful part of 1.28.6 for us is updating `ext/a-brotli`, not MuPDF itself |
| `master` | Version string **1.29.0**, not released; 418 commits not in 1.28.6 |

Re-check: `git ls-remote --tags https://github.com/ArtifexSoftware/mupdf.git` and
`gh api repos/ArtifexSoftware/mupdf/compare/1.28.5...1.28.6`.

### R1.3 The 36 patches, grouped

Numbers are SumatraPDF's; gaps are patches that were deleted when upstream MuPDF caught up (the README's example is an
older stext patch that 1.28.2 made unnecessary).

**SumatraPDF integration (tools run inside SumatraPDF.exe, Windows)**

| Patch | What |
| --- | --- |
| 0001 | Tool usage text says `SumatraPDF <tool>`, not `mutool` |
| 0002 | Tool `main`s can run more than once in one process |
| 0003 | Signing uses the Windows CryptoAPI helper instead of OpenSSL |
| 0004 | Console input and output for a GUI-subsystem exe |
| 0005 | `mutool info` output into a buffer, for the Properties window |
| 0045 | UTF-8 to a Windows console through `WriteConsoleW` |
| 0008 | An uncaught `fz_throw` reaches the crash report |
| 0007 | HarfBuzz and OpenJPEG "secret context" passing made thread-safe |

**Image formats**

| Patch | What |
| --- | --- |
| 0006 | JPEG XR through Windows WIC |
| 0025, 0027 | WebP images (libwebp), with their ICC profile |
| 0047 | JPEG XL images and PDF `/JXLDecode` |
| 0019 | FreeType built with zlib and brotli (for WOFF2 fonts) |

**Text and fonts**

| Patch | What |
| --- | --- |
| 0009 | Central European (CP1250) text in form fields |
| 0010 | ToUnicode from coded glyph names (`G45`, `g0045`, `C65`) |
| 0011 | Cyrillic Type 1 fonts with an identity-Latin ToUnicode |

**EPUB, HTML, CSS, XML, SVG, Markdown**

| Patch | What |
| --- | --- |
| 0013 | Recover from badly nested FB2 / HTML |
| 0014 | No stack overflow on deeply nested markup |
| 0015 | A user stylesheet's `!important` beats inline style |
| 0038 | CSS `background-image` and its size, position and repeat (fixed-layout scanned EPUBs were blank) |
| 0039 | Empty Markdown no longer underflows |
| 0040 | SVG `class` styles from `<style>` (such files drew all black) |
| 0044 | SVG `<image>` without width and height uses the image's size |

**PDF**

| Patch | What |
| --- | --- |
| 0012 | App callback for streams that point to an external file |
| 0032 | Placeholder appearance for Movie, Screen, 3D, RichMedia and other unrendered annotations |
| 0033 | Highlight defaults to yellow, markup without QuadPoints uses `/Rect`, Movie `/Poster` as appearance |
| 0036 | Optional content: a PrintState / ViewState usage event turns the layer on |

**Command-line tools**

| Patch | What |
| --- | --- |
| 0042 | `merge` with a backwards range (`3-1`) renumbers bookmarks too |
| 0043 | `merge` exits 1 on failure |
| 0046 | `grep` no longer reads a page it already dropped |

**Enhanced only (not in upstream SumatraPDF)**

| Patch | What |
| --- | --- |
| 0048 | Recovery snapshots keep the live incremental xref and annotation undo / redo |
| 0049 | Rounded corners on text and block highlights |
| 0050 | Zero-length border dashes draw as round dots |
| 0051 | Pinned-paper appearance for sticky notes |
| 0052 | Sharp arrowhead strokes |
| 0053 | Compact filled arrowheads and annotation-aware fill colours |

Enhanced's `0001` also differs slightly from upstream SumatraPDF's: upstream's version also covers the `mubar` tool.

### R1.4 What upstream SumatraPDF has that Enhanced lacks

Enhanced forked from upstream SumatraPDF at `a0d8bcaed` (2026-09-30). Upstream `master` (`b1a8e2257`, 2026-11-10)
still uses MuPDF 1.28.5 but now carries **70 patches**: the 30 Enhanced shares, plus **40 more**, numbered
`0048`–`0087`. Enhanced's own six are also numbered `0048`–`0053`, so **the numbers clash**: the same file name means
a different patch in each tree.

The 40 upstream-only patches, by theme:

| Theme | Patches |
| --- | --- |
| Crash and memory-safety fixes | 0050 (rearranging pages of a tagged PDF used freed memory), 0051 and 0052 (JavaScript errors on render threads), 0060 (a stream lost on a `garbage=3` save) |
| EPUB layout | 0054 (non-linear spine items), 0063 (fixed-layout pages), 0073 (footnotes as popups), 0077 (no blank page before a full-page image) |
| HTML and CSS | 0064 (CSS `transform`), 0070, 0072 (video and audio as links), 0078, 0079 (word-width cache: big books load faster), 0081, 0082, 0085 (CJK fonts and justification), 0086 (faster link loading) |
| SVG | 0055 (data URIs), 0056 (percent sizes), 0059 (`<textPath>`), 0062 (`rgba()` and `@font-face`) |
| Right-to-left text | 0080, 0083 (RTL in free text and form fields), 0084 (copying Arabic keeps order) |
| Rendering | 0057, 0058 (scalable images and display-list images), 0069 (faster type 1 shadings), 0087 (faster downscaled grey and bitonal scans) |
| Text extraction | 0061 (selection with tall-font glyphs) |
| Signing | 0053 (certifying signatures: DocMDP) |
| Passwords | 0074, 0075, 0076 (reopen with a remembered key; password support in tools) |
| Fonts | 0071 (bold fallback font cached apart from regular) |
| Free text | 0068 (rich-text box right edge) |
| Tools and scripting | 0048 (compress option), 0049, 0065 (bad options caught), 0066, 0067 (`mutool run` fixes) |

Re-check: `git fetch upstream master` in `C:\dev\sumatrapdf`, then compare `git ls-tree --name-only upstream/master
ext/patches/` with `ls ext/patches`.

### R1.5 What this means for ExcellentPDF

1. **Base the vendored tree on upstream SumatraPDF's current patch set, not Enhanced's.** Upstream's 70 patches
   include crash fixes (0050, 0051, 0052, 0060), certifying signatures (0053, relevant to O.14), RTL text (0080,
   0083, 0084, relevant to U.7) and speed fixes (0069, 0079, 0086, 0087) that Enhanced lacks.
2. **Then add Enhanced's six annotation-appearance patches** (its 0048–0053), renumbered after upstream's last
   (0088–0093), since several match ExcellentPDF's annotation work (A.1 highlights, A.8 arrows, A.5 sticky notes,
   K.15 recovery). Tested on upstream's tree: 0048, 0049, 0050 and 0052 apply cleanly; 0051 conflicts in
   `pdf-appearance.c` (upstream 0068 and 0083 touch it) and 0053 builds on 0051, so both need a hand rebase.
3. **Done 2026-11-12:** the plan (§6.1, §6.2, T2, K.2) and `AGENTS.md` D3 now say 70 + 6. Record the source commit
   of each patch in `third_party/mupdf-patches/README.md`.
4. **Stay on 1.28.5 for Phase 0.** Every MuPDF function the plan names was checked against the 1.28.5 headers, and
   1.28.6 changes nothing in MuPDF's own code that SumatraPDF builds. Its brotli bump is already covered: upstream
   SumatraPDF moved `ext/a-brotli` from Artifex commit `2523f33` (what 1.28.5 pins, and what Enhanced has) to
   `4328e14` (2026-10-27). 1.28.6 pins `05d2263`, a sibling on the same branch (the two differ by a few commits
   each way). Taking upstream's amalgamations is enough.
5. **Upstream moved its own MuPDF sources** from `src/mupdf/` to `src/shared/mupdf/` and added a macOS font
   loader, `src/ng/mupdf/mupdf_load_system_font_mac.c`, useful for ExcellentPDF's macOS phase.
6. **Plan the move to 1.29** once it is released, with SumatraPDF's update procedure in `ext/patches/README.md`
   (apply each patch with `git apply --3way`, drop the ones upstream now contains, re-verify byte for byte).
