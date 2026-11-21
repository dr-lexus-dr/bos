/* BIOS and EFI share one bounded LZMA stream and decode into the final ELF image. */
#include "LzmaDec.h"
typedef struct { ISzAlloc api; void *scratch; } BootAllocator;
static void *allocate(ISzAllocPtr api, size_t size) {
    return size <= 32768 ? ((const BootAllocator *)api)->scratch : NULL;
}
static void release(ISzAllocPtr api, void *p) { (void)api; (void)p; }
int unpack(const Byte *src, unsigned size, Byte *dst, unsigned plain, void *scratch) {
    if (size <= LZMA_PROPS_SIZE || src[0] != 0x5d) return 0;
    BootAllocator memory = {{allocate, release}, scratch};
    SizeT input = size - LZMA_PROPS_SIZE, output = plain;
    ELzmaStatus status;
    SRes result = LzmaDecode(dst, &output, src + LZMA_PROPS_SIZE, &input,
                            src, LZMA_PROPS_SIZE, LZMA_FINISH_END, &status, &memory.api);
    return result == SZ_OK && status == LZMA_STATUS_FINISHED_WITH_MARK &&
           output == plain && input == size - LZMA_PROPS_SIZE;
}
