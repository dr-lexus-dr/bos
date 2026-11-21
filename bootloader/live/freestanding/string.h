#ifndef BOS_BOOT_STRING_H
#define BOS_BOOT_STRING_H
#include <stddef.h>
static inline void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    for (size_t i = 0; i < n; i++) d[i] = s[i];
    return dst;
}
#endif
