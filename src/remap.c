#include "remap.h"
#include "debug_log.h"
#include "easyflash.h"
#include "usb_gamepad.h"
#include <stdbool.h>
#include <stddef.h>
#include <string.h>

#define REMAP_EF_KEY_V3 "btn_remap_v3"
#define REMAP_EF_KEY_V2 "btn_remap_v2"
#define REMAP_STORE_MAGIC 0x33504D52u /* RMP3 */

typedef struct {
    uint32_t magic;
    uint16_t revision;
    uint8_t version;
    uint8_t count;
    uint32_t masks[REMAP_BTN_COUNT];
    uint32_t checksum;
} remap_store_t;

/* Retained v2 layout. The original key is not changed so rollback keeps the
 * pre-upgrade mapping. */
typedef struct {
    uint8_t type;
    uint8_t value;
    uint8_t modifier;
    uint8_t flags;
} remap_v2_entry_t;

static const struct { uint8_t off; uint8_t mask; } BTN_LOC[REMAP_DIGITAL_BTN_COUNT] = {
    [REMAP_BTN_SQUARE] = { 7, 0x10 }, [REMAP_BTN_CROSS] = { 7, 0x20 },
    [REMAP_BTN_CIRCLE] = { 7, 0x40 }, [REMAP_BTN_TRIANGLE] = { 7, 0x80 },
    [REMAP_BTN_L1] = { 8, 0x01 }, [REMAP_BTN_R1] = { 8, 0x02 },
    [REMAP_BTN_L2] = { 8, 0x04 }, [REMAP_BTN_R2] = { 8, 0x08 },
    [REMAP_BTN_CREATE] = { 8, 0x10 }, [REMAP_BTN_OPTIONS] = { 8, 0x20 },
    [REMAP_BTN_L3] = { 8, 0x40 }, [REMAP_BTN_R3] = { 8, 0x80 },
    [REMAP_BTN_PS] = { 9, 0x01 }, [REMAP_BTN_TP_CLICK] = { 9, 0x02 },
    [REMAP_BTN_MUTE] = { 9, 0x04 },
};

static uint32_t g_masks[REMAP_BTN_COUNT];
static uint16_t g_revision;
static bool g_identity;

static uint32_t checksum32(const uint8_t *data, size_t len)
{
    uint32_t hash = 2166136261u;
    for (size_t i = 0; i < len; ++i) {
        hash ^= data[i];
        hash *= 16777619u;
    }
    return hash;
}

static void update_identity(void)
{
    g_identity = true;
    for (uint8_t i = 0; i < REMAP_BTN_COUNT; ++i) {
        if (g_masks[i] != (1u << i)) {
            g_identity = false;
            break;
        }
    }
}

static bool masks_valid(const uint32_t *masks)
{
    if (masks == NULL) return false;
    for (uint8_t i = 0; i < REMAP_BTN_COUNT; ++i) {
        if ((masks[i] & ~REMAP_TARGET_MASK) != 0) return false;
    }
    return true;
}

static inline uint8_t get_src_bit(const uint8_t *p, int id)
{
    if (id < REMAP_DIGITAL_BTN_COUNT)
        return (p[BTN_LOC[id].off] & BTN_LOC[id].mask) ? 1u : 0u;
    const uint8_t hat = p[7] & 0x0Fu;
    switch (id) {
    case REMAP_BTN_DPAD_UP: return hat == 0 || hat == 1 || hat == 7;
    case REMAP_BTN_DPAD_RIGHT: return hat == 1 || hat == 2 || hat == 3;
    case REMAP_BTN_DPAD_DOWN: return hat == 3 || hat == 4 || hat == 5;
    case REMAP_BTN_DPAD_LEFT: return hat == 5 || hat == 6 || hat == 7;
    default: return 0;
    }
}

static uint8_t encode_hat(uint8_t up, uint8_t right, uint8_t down, uint8_t left)
{
    if (up && down) up = down = 0;
    if (left && right) left = right = 0;
    if (up) return right ? 1 : (left ? 7 : 0);
    if (down) return right ? 3 : (left ? 5 : 4);
    if (right) return 2;
    if (left) return 6;
    return 8;
}

void remap_init(void)
{
    g_revision = 0;
    remap_reset();
}

void remap_reset(void)
{
    for (uint8_t i = 0; i < REMAP_BTN_COUNT; ++i) g_masks[i] = 1u << i;
    if (++g_revision == 0) g_revision = 1;
    g_identity = true;
}

void remap_load(void)
{
    remap_store_t stored;
    size_t len = 0;
    memset(&stored, 0, sizeof(stored));
    ef_get_env_blob(REMAP_EF_KEY_V3, &stored, sizeof(stored), &len);
    const uint32_t expected = checksum32((const uint8_t *)&stored,
                                          offsetof(remap_store_t, checksum));
    if (len == sizeof(stored) && stored.magic == REMAP_STORE_MAGIC &&
        stored.version == REMAP_WIRE_VERSION && stored.count == REMAP_BTN_COUNT &&
        stored.checksum == expected && masks_valid(stored.masks)) {
        memcpy(g_masks, stored.masks, sizeof(g_masks));
        g_revision = stored.revision ? stored.revision : 1;
        update_identity();
        LOG_INF("[REMAP] Loaded v3 revision %u, identity=%d\n",
                g_revision, (int)g_identity);
        return;
    }

    remap_v2_entry_t old[REMAP_BTN_COUNT];
    memset(old, 0, sizeof(old));
    len = 0;
    ef_get_env_blob(REMAP_EF_KEY_V2, old, sizeof(old), &len);
    if (len > 0 && len <= sizeof(old) && len % sizeof(old[0]) == 0) {
        remap_reset();
        const size_t count = len / sizeof(old[0]);
        for (size_t i = 0; i < count; ++i) {
            if (old[i].type == 0 && old[i].value < REMAP_BTN_COUNT)
                g_masks[i] = 1u << old[i].value;
        }
        update_identity();
        (void)remap_save();
        LOG_INF("[REMAP] Migrated %u v2 entries to v3\n", (unsigned)count);
        return;
    }

    remap_reset();
    LOG_INF("[REMAP] No valid mapping found; using identity v3\n");
}

bool remap_save(void)
{
    remap_store_t stored = {
        .magic = REMAP_STORE_MAGIC,
        .revision = g_revision,
        .version = REMAP_WIRE_VERSION,
        .count = REMAP_BTN_COUNT,
    };
    memcpy(stored.masks, g_masks, sizeof(g_masks));
    stored.checksum = checksum32((const uint8_t *)&stored,
                                  offsetof(remap_store_t, checksum));
    return ef_set_env_blob(REMAP_EF_KEY_V3, &stored, sizeof(stored)) == 0;
}

bool remap_set_masks(const uint32_t *masks, uint8_t count,
                     uint16_t expected_revision)
{
    if (count != REMAP_BTN_COUNT || !masks_valid(masks)) return false;
    if (expected_revision != 0xFFFFu && expected_revision != g_revision) return false;
    memcpy(g_masks, masks, sizeof(g_masks));
    if (++g_revision == 0) g_revision = 1;
    update_identity();
    return true;
}

void remap_get_masks(uint32_t *masks, uint8_t count)
{
    if (masks == NULL) return;
    const uint8_t copy = count < REMAP_BTN_COUNT ? count : REMAP_BTN_COUNT;
    memcpy(masks, g_masks, copy * sizeof(g_masks[0]));
}

uint16_t remap_revision(void) { return g_revision; }

uint32_t remap_physical_mask(const uint8_t *payload)
{
    uint32_t mask = 0;
    if (payload == NULL) return 0;
    for (uint8_t i = 0; i < REMAP_BTN_COUNT; ++i) {
        if (get_src_bit(payload, i)) mask |= 1u << i;
    }
    return mask;
}

uint32_t remap_logical_mask(const uint8_t *payload)
{
    return remap_physical_mask(payload);
}

void remap_write_logical_mask(uint8_t *p, uint32_t mask,
                              uint8_t l2_value, uint8_t r2_value)
{
    if (p == NULL) return;
    mask &= REMAP_TARGET_MASK;
    p[4] = (mask & (1u << REMAP_BTN_L2)) ? l2_value : 0;
    p[5] = (mask & (1u << REMAP_BTN_R2)) ? r2_value : 0;
    p[7] = encode_hat((mask >> REMAP_BTN_DPAD_UP) & 1u,
                      (mask >> REMAP_BTN_DPAD_RIGHT) & 1u,
                      (mask >> REMAP_BTN_DPAD_DOWN) & 1u,
                      (mask >> REMAP_BTN_DPAD_LEFT) & 1u)
         | ((mask & (1u << REMAP_BTN_SQUARE)) ? 0x10 : 0)
         | ((mask & (1u << REMAP_BTN_CROSS)) ? 0x20 : 0)
         | ((mask & (1u << REMAP_BTN_CIRCLE)) ? 0x40 : 0)
         | ((mask & (1u << REMAP_BTN_TRIANGLE)) ? 0x80 : 0);
    p[8] = ((mask & (1u << REMAP_BTN_L1)) ? 0x01 : 0)
         | ((mask & (1u << REMAP_BTN_R1)) ? 0x02 : 0)
         | ((mask & (1u << REMAP_BTN_L2)) ? 0x04 : 0)
         | ((mask & (1u << REMAP_BTN_R2)) ? 0x08 : 0)
         | ((mask & (1u << REMAP_BTN_CREATE)) ? 0x10 : 0)
         | ((mask & (1u << REMAP_BTN_OPTIONS)) ? 0x20 : 0)
         | ((mask & (1u << REMAP_BTN_L3)) ? 0x40 : 0)
         | ((mask & (1u << REMAP_BTN_R3)) ? 0x80 : 0);
    p[9] = (p[9] & 0xF8)
         | ((mask & (1u << REMAP_BTN_PS)) ? 0x01 : 0)
         | ((mask & (1u << REMAP_BTN_TP_CLICK)) ? 0x02 : 0)
         | ((mask & (1u << REMAP_BTN_MUTE)) ? 0x04 : 0);
}

void remap_apply(uint8_t *p)
{
    if (g_identity) return;
    uint8_t src[REMAP_BTN_COUNT];
    for (uint8_t i = 0; i < REMAP_BTN_COUNT; ++i) src[i] = get_src_bit(p, i);
    const uint8_t analog_l2 = p[4], analog_r2 = p[5];
    uint8_t dst[REMAP_BTN_COUNT] = { 0 };
    uint8_t out_l2 = 0, out_r2 = 0;

    for (uint8_t source = 0; source < REMAP_BTN_COUNT; ++source) {
        if (!src[source]) continue;
        const uint32_t mask = g_masks[source];
        for (uint8_t target = 0; target < REMAP_BTN_COUNT; ++target) {
            if ((mask & (1u << target)) == 0) continue;
            dst[target] = 1;
            const uint8_t value = source == REMAP_BTN_L2 ? analog_l2 :
                                  source == REMAP_BTN_R2 ? analog_r2 : 0xFF;
            if (target == REMAP_BTN_L2 && value > out_l2) out_l2 = value;
            if (target == REMAP_BTN_R2 && value > out_r2) out_r2 = value;
        }
    }

    p[4] = out_l2;
    p[5] = out_r2;
    p[7] = encode_hat(dst[REMAP_BTN_DPAD_UP], dst[REMAP_BTN_DPAD_RIGHT],
                      dst[REMAP_BTN_DPAD_DOWN], dst[REMAP_BTN_DPAD_LEFT])
         | (dst[REMAP_BTN_SQUARE] ? 0x10 : 0)
         | (dst[REMAP_BTN_CROSS] ? 0x20 : 0)
         | (dst[REMAP_BTN_CIRCLE] ? 0x40 : 0)
         | (dst[REMAP_BTN_TRIANGLE] ? 0x80 : 0);
    p[8] = (dst[REMAP_BTN_L1] ? 0x01 : 0) | (dst[REMAP_BTN_R1] ? 0x02 : 0)
         | (dst[REMAP_BTN_L2] ? 0x04 : 0) | (dst[REMAP_BTN_R2] ? 0x08 : 0)
         | (dst[REMAP_BTN_CREATE] ? 0x10 : 0) | (dst[REMAP_BTN_OPTIONS] ? 0x20 : 0)
         | (dst[REMAP_BTN_L3] ? 0x40 : 0) | (dst[REMAP_BTN_R3] ? 0x80 : 0);
    p[9] = (p[9] & 0xF8) | (dst[REMAP_BTN_PS] ? 0x01 : 0)
         | (dst[REMAP_BTN_TP_CLICK] ? 0x02 : 0) | (dst[REMAP_BTN_MUTE] ? 0x04 : 0);
}

void remap_kbd_tick(const uint8_t *p) { (void)p; }
void remap_on_disconnect(void) { }
bool remap_has_kbd_targets(void) { return false; }
