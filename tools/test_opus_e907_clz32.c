#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

static unsigned clz32_reference(uint32_t value)
{
    unsigned count = 0;

    if (value == 0)
        return 32;
    while ((value & UINT32_C(0x80000000)) == 0) {
        ++count;
        value <<= 1;
    }
    return count;
}

static unsigned clz32_model(uint32_t value)
{
    return value == 0 ? 32U : (unsigned)__builtin_clz(value);
}

static int check_one(uint32_t value)
{
    unsigned reference = clz32_reference(value);
    unsigned candidate = clz32_model(value);

    if (reference == candidate)
        return 0;
    fprintf(stderr,
            "mismatch value=%" PRIu32 " reference=%u candidate=%u\n",
            value, reference, candidate);
    return 1;
}

int main(void)
{
    static const uint32_t edges[] = {
        0, 1, 2, 3, 7, 8, 15, 16, UINT16_MAX,
        UINT16_MAX + UINT32_C(1), INT32_MAX,
        UINT32_C(0x80000000), UINT32_MAX,
    };
    uint32_t state = UINT32_C(0xc123e907);
    uint64_t checked = 0;
    size_t i;

    for (i = 0; i < sizeof(edges) / sizeof(edges[0]); ++i) {
        if (check_one(edges[i]))
            return 1;
        ++checked;
    }
    for (i = 0; i < 1000000; ++i) {
        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        if (check_one(state))
            return 1;
        ++checked;
    }
    printf("clz32 bit-exact: checked=%" PRIu64 "\n", checked);
    return 0;
}
