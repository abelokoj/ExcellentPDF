# Include anchor

`noto_sumatra.c` includes `"../../../ext/mupdf/source/fitz/font-table.h"`, a path relative to SumatraPDF's
`src/shared/mupdf/`. The build adds this folder to the include path, so the path resolves to
`sumatra-layout/ext/mupdf/source/fitz/font-table.h`.

Windows resolves `..` by text, but macOS and Linux need every folder in the path to exist. Git keeps no empty
folders, so this file keeps the folder in the repository. Don't delete it.
