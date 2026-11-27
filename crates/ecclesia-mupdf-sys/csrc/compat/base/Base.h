/* Stand-in for SumatraPDF's src/base/Base.h, which third_party/sumatra/shared/mupdf/load-jxl.cpp includes. That
 * file uses only standard C++ from it, so the standard headers are enough; we don't build SumatraPDF's base library. */
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <string.h>
