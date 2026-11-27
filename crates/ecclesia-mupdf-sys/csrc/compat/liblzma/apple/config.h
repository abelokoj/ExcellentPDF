/* liblzma's macOS config.h, also used for iOS. third_party/amalgamations/liblzma/config.h is the Windows one;
 * this directory goes before liblzma's on the include path. */
#include "config_macos.h"

/* config_macos.h assumes a 64-bit target; 32-bit Android and simulators have a 4-byte size_t. */
#undef SIZEOF_SIZE_T
#define SIZEOF_SIZE_T __SIZEOF_SIZE_T__
