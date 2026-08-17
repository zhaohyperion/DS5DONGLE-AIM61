#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

static int32_t q15_reference(int16_t a, int32_t b)
{
    int32_t high = b >> 16;
    uint32_t low = (uint16_t)b;

    /* This is the non-OPUS_FAST_INT64 expression in fixed_generic.h. */
    return (int32_t)((int64_t)high * a * 2 +
                     (((int64_t)a * low) >> 15));
}

static int32_t q15_kmmwb2_model(int16_t a, int32_t b)
{
    if (a == INT16_MIN && b == INT32_MIN)
        return INT32_MAX;
    return (int32_t)(((int64_t)b * a) >> 15);
}

static int check_one(int16_t a, int32_t b, uint64_t *invalid_domain)
{
    int32_t reference;
    int32_t candidate;

    if (a == INT16_MIN && b == INT32_MIN) {
        ++*invalid_domain;
        return q15_kmmwb2_model(a, b) == INT32_MAX ? 0 : 1;
    }
    reference = q15_reference(a, b);
    candidate = q15_kmmwb2_model(a, b);
    if (reference == candidate)
        return 0;
    fprintf(stderr,
            "mismatch a=%" PRId16 " b=%" PRId32 " reference=%" PRId32
            " candidate=%" PRId32 "\n",
            a, b, reference, candidate);
    return 1;
}

int main(void)
{
    static const int16_t a_edges[] = {
        INT16_MIN, INT16_MIN + 1, -32767, -1, 0, 1, 32766, INT16_MAX,
    };
    static const int32_t b_edges[] = {
        INT32_MIN, INT32_MIN + 1, -65537, -65536, -65535, -1,
        0, 1, 65535, 65536, 65537, INT32_MAX - 1, INT32_MAX,
    };
    uint32_t state = UINT32_C(0x15e90732);
    uint64_t checked = 0;
    uint64_t invalid_domain = 0;
    size_t i;
    size_t j;

    for (i = 0; i < sizeof(a_edges) / sizeof(a_edges[0]); ++i) {
        for (j = 0; j < sizeof(b_edges) / sizeof(b_edges[0]); ++j) {
            if (check_one(a_edges[i], b_edges[j], &invalid_domain))
                return 1;
            ++checked;
        }
    }
    for (i = 0; i < 1000000; ++i) {
        int16_t a;
        int32_t b;

        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        a = (int16_t)(state >> 16);
        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        b = (int32_t)state;
        if (check_one(a, b, &invalid_domain))
            return 1;
        ++checked;
    }
    printf("q15-kmmwb2 exact in documented domain: checked=%" PRIu64
           " excluded_overflow=%" PRIu64 "\n",
           checked, invalid_domain);
    return 0;
}
