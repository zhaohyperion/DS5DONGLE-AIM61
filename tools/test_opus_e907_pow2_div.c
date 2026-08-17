#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

static unsigned ilog32(uint32_t value)
{
    unsigned bits = 0;

    do {
        ++bits;
        value >>= 1;
    } while (value != 0);
    return bits;
}

static uint32_t pow2_div_model(uint32_t numerator, uint32_t denominator)
{
    return numerator >> (ilog32(denominator) - 1U);
}

static int check_one(uint32_t numerator, uint32_t denominator)
{
    uint32_t reference = numerator / denominator;
    uint32_t candidate = pow2_div_model(numerator, denominator);

    if (reference == candidate)
        return 0;
    fprintf(stderr,
            "mismatch n=%" PRIu32 " d=%" PRIu32
            " reference=%" PRIu32 " candidate=%" PRIu32 "\n",
            numerator, denominator, reference, candidate);
    return 1;
}

int main(void)
{
    static const uint32_t edges[] = {
        0, 1, 2, 3, 7, 8, 15, 16, 31, 32,
        UINT16_MAX, UINT16_MAX + UINT32_C(1),
        INT32_MAX, UINT32_C(0x80000000), UINT32_MAX,
    };
    uint32_t state = UINT32_C(0xe907d1a5);
    uint64_t checked = 0;
    unsigned shift;
    size_t i;

    for (shift = 0; shift < 32; ++shift) {
        uint32_t denominator = UINT32_C(1) << shift;

        for (i = 0; i < sizeof(edges) / sizeof(edges[0]); ++i) {
            if (check_one(edges[i], denominator))
                return 1;
            ++checked;
        }
        for (i = 0; i < 100000; ++i) {
            state = state * UINT32_C(1664525) + UINT32_C(1013904223);
            if (check_one(state, denominator))
                return 1;
            ++checked;
        }
    }
    printf("pow2-div bit-exact: checked=%" PRIu64 "\n", checked);
    return 0;
}
