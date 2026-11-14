/* liblzma's Linux config.h, also used for Android. third_party/amalgamations/liblzma/config.h is the Windows one;
 * this directory goes before liblzma's on the include path. */
#include "config_linux.h"

/* config_linux.h assumes a 64-bit target; 32-bit Android and simulators have a 4-byte size_t. */
#undef SIZEOF_SIZE_T
#define SIZEOF_SIZE_T __SIZEOF_SIZE_T__
