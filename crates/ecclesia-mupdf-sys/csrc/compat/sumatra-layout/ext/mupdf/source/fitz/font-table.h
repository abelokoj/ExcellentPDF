/* noto_sumatra.c includes "../../../ext/mupdf/source/fitz/font-table.h", a path relative to SumatraPDF's tree
 * (src/shared/mupdf/ -> ext/mupdf/). build.rs adds the include path sumatra-layout/src/shared/mupdf (a README keeps
 * that folder in git: macOS and Linux resolve ".." only through folders that exist), so that include lands here, and
 * this forwards it to MuPDF's real table through the normal include paths, which have third_party/mupdf/source/fitz.
 */
#include <font-table.h>
