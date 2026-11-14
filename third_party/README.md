# Third-party code

Everything here is copied from other projects and built by `crates/excellent-mupdf-sys`. Don't edit it in place:
MuPDF changes go through `mupdf-patches/`; the other folders are replaced wholesale when they are updated.

| Folder | What | Source |
| --- | --- | --- |
| `mupdf/` | MuPDF **1.28.5** (tag `1.28.5`, commit `8ad45e92f`, released 2026-09-24), the 1410-file subset SumatraPDF vendors, with all 76 patches applied | Upstream SumatraPDF `ext/mupdf` |
| `mupdf-patches/` | 70 patches from upstream SumatraPDF (`0001`–`0087`) and six from SumatraPDF Enhanced (`0088`–`0093`); its README records each one's origin and the verification | Upstream SumatraPDF `ext/patches`, Enhanced `ext/patches` |
| `amalgamations/` | The libraries MuPDF needs, mostly one `.c` file each: `a-brotli`, `a-extract`, `a-freetype`, `a-gumbo`, `a-harfbuzz`, `a-jbig2dec`, `a-lcms2`, `a-libwebp`, `a-mujs`, `a-openjpeg`, `a-zlib`, plus `libjpeg-turbo`, `cmark-gfm` and `jxldec` (for patch `0047`); OCR: `tesseract` 5.5.2 and `leptonica` 1.87.0; barcodes: `zxing-cpp` 2.3.0 and `zint` 2.13; archives: `a-libarchive` 3.8.8, `a-bzip2` 1.0.8 and `liblzma` (xz 5.8.3). Versions in `amalgamations/versions.txt` and each folder's `version.txt` | Upstream SumatraPDF `ext/` |
| `sumatra/shared/mupdf/` | SumatraPDF's own C files compiled into its `mupdf` library: `noto_sumatra.c` (built-in fonts loaded by file name), `mupdf_load_system_font.c` (system font fallback and HarfBuzz allocators), `load-jxl`, `pkcs7-windows`, and others we may not build | Upstream SumatraPDF `src/shared/mupdf` |
| `sumatra/ng/mupdf/` | SumatraPDF's macOS and POSIX counterparts (`mupdf_load_system_font_mac.c`, `pkcs7_mac`, `pkcs7_stub_posix.c`), for the macOS and Linux phases | Upstream SumatraPDF `src/ng/mupdf` |
| `sumatra/ng/libarchive/` | `libarchive_posix.c`, libarchive's POSIX-only files, built on every target but Windows | Upstream SumatraPDF `src/ng/libarchive` |

## Source revisions

| Source | Revision |
| --- | --- |
| Upstream SumatraPDF (`sumatrapdfreader/sumatrapdf`, `master`) | `b1a8e22577c9e35064315d279fc33ba6b67c5bef`, 2026-11-10 |
| SumatraPDF Enhanced (`C:\dev\sumatrapdf`, `HEAD`) | `22a10038ae6c1c4e08ad91190ebfbd7d47dfab9c`, 2026-11-11 (patches `0088`–`0093` only) |
| MuPDF (`ArtifexSoftware/mupdf`, tag `1.28.5`) | `8ad45e92f0935d3d87f1db3f873086472a5e1b24`, used only to verify the tree |

Copied on 2026-11-13 (task T2) with
`git archive b1a8e2257 ext/mupdf ext/patches ext/versions.txt src/shared/mupdf src/ng/mupdf ext/a-brotli ext/a-extract
ext/a-freetype ext/a-gumbo ext/a-harfbuzz ext/a-jbig2dec ext/a-lcms2 ext/a-libwebp ext/a-mujs ext/a-openjpeg ext/a-zlib
ext/libjpeg-turbo ext/cmark-gfm ext/jxldec`; on the same day, for `dev notes/dev_decisions.md` DD1 (all of MuPDF),
`git archive b1a8e2257 ext/tesseract ext/leptonica ext/zxing-cpp ext/zint ext/a-libarchive ext/a-bzip2 ext/liblzma
src/ng/libarchive`.

## Updating

1. In `C:\dev\sumatrapdf`, `git fetch upstream master` and export the same paths from the new commit.
2. Replace `mupdf/` with upstream's `ext/mupdf`, then apply `0088`–`0093` on top (`git apply`, LF line endings).
3. Copy upstream's new or changed patches into `mupdf-patches/`, update the README's origin table, and run the byte
   comparison it describes.
4. Replace the library folders and `versions.txt`; update this file's revisions.

## Licences

Each library folder keeps its upstream licence file (`LICENSE`, `COPYING` or its source headers). MuPDF and
`noto_sumatra.c` are AGPL-3.0. SumatraPDF's own files are GPL-3.0 (its `COPYING`) unless their header says otherwise:
`pkcs7-windows.c` is Simplified BSD. The libraries are under permissive licences (FreeType, HarfBuzz, zlib,
libjpeg-turbo, and so on): Tesseract and zxing-cpp Apache-2.0, Leptonica BSD-2-Clause style, zint BSD-3-Clause,
libarchive BSD-2-Clause, bzip2 its own BSD-style licence, liblzma 0BSD. ExcellentPDF's own licence is open question Q1 (plan §21, §4).
