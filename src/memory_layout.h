#ifndef DS5_MEMORY_LAYOUT_H
#define DS5_MEMORY_LAYOUT_H

#include "compiler/compiler_ld.h"

/*
 * External PSRAM is intentionally reserved for data that is both cold and
 * explicitly initialized by the application.  Never apply this attribute to
 * USB endpoint buffers, Bluetooth TX queues, Opus state, or any other object
 * on the 1 ms audio/input path.
 *
 * .psram_noinit avoids copying/clearing a large section during startup.  Every
 * annotated object must therefore have an explicit lifecycle clear before its
 * first read.
 */
#ifdef CONFIG_PSRAM
#define DS5_PSRAM_COLD_DATA \
    ATTR_NOINIT_PSRAM_SECTION __attribute__((aligned(32)))
#else
#define DS5_PSRAM_COLD_DATA
#endif

#endif /* DS5_MEMORY_LAYOUT_H */
