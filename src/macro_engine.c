#include "macro_engine.h"

#include "debug_log.h"
#include "easyflash.h"
#include "memory_layout.h"
#include "remap.h"
#include "led_status.h"
#include "FreeRTOS.h"
#include "queue.h"
#include "task.h"
#include "bflb_mtimer.h"
#include <stddef.h>
#include <string.h>

#define MACRO_STORE_MAGIC 0x3354534Du /* MST3 */
#define MACRO_BLOB_MAGIC  0x3343414Du /* MAC3 */
#define MACRO_STORE_KEY_A "macro_store_a"
#define MACRO_STORE_KEY_B "macro_store_b"
#define MACRO_HEADER_SIZE 32u
#define MACRO_DESC_SIZE   24u
#define MACRO_MAX_COUNT   76u
#define MACRO_MAX_STEPS   1024u
#define MACRO_QUEUE_DEPTH 4u
#define MACRO_RECORD_BYTES 7600u
#define MACRO_RECORD_WARN_BYTES (MACRO_RECORD_BYTES * 4u / 5u)

enum macro_command {
    MACRO_CMD_BEGIN       = 0x01,
    MACRO_CMD_CHUNK       = 0x02,
    MACRO_CMD_COMMIT      = 0x03,
    MACRO_CMD_ABORT       = 0x04,
    MACRO_CMD_READ_SELECT = 0x05,
    MACRO_CMD_CONTROL     = 0x06,
    MACRO_CMD_STOP        = 0x07,
};

enum macro_state {
    MACRO_STATE_IDLE      = 0,
    MACRO_STATE_RECEIVING = 1,
    MACRO_STATE_READY     = 2,
    MACRO_STATE_ERROR     = 3,
};

enum macro_error {
    MACRO_ERROR_NONE       = 0,
    MACRO_ERROR_FRAME      = 1,
    MACRO_ERROR_STALE      = 2,
    MACRO_ERROR_BOUNDS     = 3,
    MACRO_ERROR_ORDER      = 4,
    MACRO_ERROR_CRC        = 5,
    MACRO_ERROR_FORMAT     = 6,
    MACRO_ERROR_STORAGE    = 7,
    MACRO_ERROR_QUEUE_FULL = 8,
};

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t generation;
    uint16_t length;
    uint16_t reserved;
    uint32_t crc32;
    uint8_t data[MACRO_MAX_BYTES];
} macro_store_t;

typedef struct {
    uint8_t payload[MACRO_REPORT_SIZE];
} macro_command_frame_t;

typedef struct {
    bool active;
    uint16_t descriptor_index;
    uint16_t cursor;
    uint16_t program_end;
    uint16_t repeats_left;
    uint64_t deadline_us;
    uint32_t digital_mask;
    uint8_t axes[6];
    uint8_t axis_valid;
    uint8_t touches[10];
    bool touch_valid;
    bool trigger_held;
} macro_playback_t;

enum recorder_state {
    RECORDER_IDLE = 0,
    RECORDER_SELECT_TRIGGER,
    RECORDER_COUNTDOWN,
    RECORDER_ACTIVE,
    RECORDER_SAVED,
};

typedef struct {
    uint32_t mask;
    uint8_t axes[6];
    uint8_t touches[10];
} recorded_state_t;

static uint8_t g_active_blob[MACRO_MAX_BYTES] DS5_PSRAM_COLD_DATA;
static macro_store_t g_transfer DS5_PSRAM_COLD_DATA;
static uint16_t g_active_len;
static uint32_t g_generation;
static uint8_t g_active_slot;
static uint16_t g_received;
static uint16_t g_expected_len;
static uint32_t g_expected_crc;
static uint32_t g_pending_generation;
static uint16_t g_read_offset;
static uint8_t g_state;
static uint8_t g_error;
static uint32_t g_raw_mask;
static uint32_t g_previous_raw_mask;
static uint64_t g_press_us[REMAP_BTN_COUNT];
static uint64_t g_last_release_us[REMAP_BTN_COUNT];
static uint32_t g_long_fired_mask;
static uint32_t g_rng = 0x61A1C35Du;
static uint64_t g_ps_press_us;
static bool g_ps_emergency_fired;
static macro_playback_t g_playback;
static QueueHandle_t g_command_queue;
static uint8_t g_status[MACRO_REPORT_SIZE];
static uint8_t g_record_program[MACRO_RECORD_BYTES] DS5_PSRAM_COLD_DATA;
static recorded_state_t g_record_previous;
static uint16_t g_record_len;
static uint16_t g_record_steps;
static uint8_t g_record_trigger;
static uint8_t g_recorder_state;
static uint64_t g_recorder_deadline_us;
static uint64_t g_record_last_us;
static uint64_t g_record_chord_press_us;
static bool g_record_chord_latched;
static bool g_record_have_state;
static bool g_record_finalize_pending;
static bool g_record_warned;
static enum led_pattern g_record_restore_led;
static bool g_management_mode;
static bool g_management_wait_release;
static bool g_management_wait_trigger;
static bool g_management_dirty;
static uint64_t g_management_chord_press_us;
static enum led_pattern g_management_restore_led;

static bool validate_blob(const uint8_t *blob, uint16_t length);
static bool persist_transfer(void);
static const uint8_t *descriptor_at(uint16_t index);

static uint16_t read16(const uint8_t *p)
{
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

static uint32_t read24(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16);
}

static uint32_t read32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static void write16(uint8_t *p, uint16_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
}

static void write32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

static uint32_t crc32_ieee(const uint8_t *data, uint32_t len)
{
    uint32_t crc = 0xFFFFFFFFu;
    while (len--) {
        crc ^= *data++;
        for (uint8_t bit = 0; bit < 8; ++bit)
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
    }
    return ~crc;
}

static uint8_t blob_flags(void)
{
    return g_active_len >= MACRO_HEADER_SIZE ? g_active_blob[7] : 0;
}

static uint8_t blob_profile(void)
{
    return g_active_len >= MACRO_HEADER_SIZE ? g_active_blob[6] : 0;
}

static void publish_status(void)
{
    uint8_t snapshot[MACRO_REPORT_SIZE] = {0};
    snapshot[0] = 'M';
    snapshot[1] = 'C';
    snapshot[2] = MACRO_FORMAT_VERSION;
    snapshot[3] = g_state;
    snapshot[4] = g_error;
    snapshot[5] = blob_flags();
    snapshot[6] = blob_profile();
    snapshot[7] = (g_playback.active ? 1u : 0u) |
                  (g_management_mode ? 0x02u : 0u) |
                  (g_recorder_state == RECORDER_ACTIVE ? 0x04u : 0u);
    write32(snapshot + 8, g_generation);
    write16(snapshot + 12, g_active_len);
    write16(snapshot + 14, g_read_offset);
    snapshot[16] = 0;
    if (g_read_offset < g_active_len) {
        uint16_t remain = g_active_len - g_read_offset;
        uint8_t count = remain > 46u ? 46u : (uint8_t)remain;
        snapshot[16] = count;
        memcpy(snapshot + 17, g_active_blob + g_read_offset, count);
    }
    taskENTER_CRITICAL();
    memcpy(g_status, snapshot, sizeof(g_status));
    taskEXIT_CRITICAL();
}

static void set_error(uint8_t error)
{
    g_error = error;
    g_state = MACRO_STATE_ERROR;
    publish_status();
}

static void recorder_restore_led(void)
{
    led_status_unlock();
    led_status_set(g_record_restore_led);
}

static void recorder_cancel(void)
{
    g_recorder_state = RECORDER_IDLE;
    g_record_len = g_record_steps = 0;
    g_record_have_state = false;
    g_record_finalize_pending = false;
    g_record_chord_latched = true;
    recorder_restore_led();
    LOG_INF("[MACRO] Standalone recording cancelled\n");
}

static bool recorder_append_state(const recorded_state_t *state, uint32_t duration_ms)
{
    while (duration_ms) {
        const uint16_t slice = duration_ms > 65535u ? 65535u : (uint16_t)duration_ms;
        if ((uint32_t)g_record_len + 27u > MACRO_RECORD_BYTES) return false;
        uint8_t *out = g_record_program + g_record_len;
        *out++ = 0x01;
        *out++ = (uint8_t)state->mask;
        *out++ = (uint8_t)(state->mask >> 8);
        *out++ = (uint8_t)(state->mask >> 16);
        *out++ = 0x02;
        memcpy(out, state->axes, 6);
        out += 6;
        *out++ = 0x03;
        memcpy(out, state->touches, 10);
        out += 10;
        *out++ = 0x04;
        write16(out, slice);
        write16(out + 2, 0);
        g_record_len += 27;
        g_record_steps++;
        duration_ms -= slice;
    }
    return true;
}

static bool recorder_build_blob(void)
{
    const uint16_t old_count = g_active_len >= MACRO_HEADER_SIZE ?
                               read16(g_active_blob + 12) : 0;
    const uint16_t old_desc_len = g_active_len >= MACRO_HEADER_SIZE ?
                                  read16(g_active_blob + 16) : 0;
    const uint16_t old_program_len = g_active_len >= MACRO_HEADER_SIZE ?
                                     read16(g_active_blob + 18) : 0;
    const uint8_t profile = blob_profile();
    uint16_t kept = 0;
    uint32_t next_id = 1;
    for (uint16_t i = 0; i < old_count; ++i) {
        const uint8_t *d = descriptor_at(i);
        const uint32_t id = read32(d);
        if (id >= next_id) next_id = id + 1u;
        if (!(d[4] == profile && d[5] == g_record_trigger)) kept++;
    }
    const uint16_t new_count = kept + 1u;
    const uint16_t new_desc_len = new_count * MACRO_DESC_SIZE;
    if ((uint32_t)MACRO_HEADER_SIZE + new_desc_len + old_program_len +
        g_record_len > MACRO_MAX_BYTES) return false;

    memset(g_transfer.data, 0, MACRO_HEADER_SIZE);
    memcpy(g_transfer.data, "MAC3", 4);
    g_transfer.data[4] = MACRO_FORMAT_VERSION;
    g_transfer.data[5] = 4;
    g_transfer.data[6] = profile;
    g_transfer.data[7] = blob_flags() | 0x03u;
    uint16_t out_desc = 0, out_program = 0, total_steps = 0;
    const uint8_t *old_program = g_active_blob + MACRO_HEADER_SIZE + old_desc_len;
    for (uint16_t i = 0; i < old_count; ++i) {
        const uint8_t *source = descriptor_at(i);
        if (source[4] == profile && source[5] == g_record_trigger) continue;
        uint8_t *dest = g_transfer.data + MACRO_HEADER_SIZE + out_desc;
        memcpy(dest, source, MACRO_DESC_SIZE);
        const uint16_t source_offset = read16(source + 18);
        const uint16_t source_len = read16(source + 20);
        write16(dest + 18, out_program);
        memcpy(g_transfer.data + MACRO_HEADER_SIZE + new_desc_len + out_program,
               old_program + source_offset, source_len);
        out_program += source_len;
        total_steps += read16(source + 16);
        out_desc += MACRO_DESC_SIZE;
    }
    uint8_t *d = g_transfer.data + MACRO_HEADER_SIZE + out_desc;
    write32(d, next_id);
    d[4] = profile;
    d[5] = g_record_trigger;
    d[6] = 0; /* physical press */
    d[7] = 0; /* one-shot */
    d[8] = 0x05; /* enabled + recorded on device */
    write16(d + 12, 1);
    write16(d + 16, g_record_steps);
    write16(d + 18, out_program);
    write16(d + 20, g_record_len);
    memcpy(g_transfer.data + MACRO_HEADER_SIZE + new_desc_len + out_program,
           g_record_program, g_record_len);
    out_program += g_record_len;
    total_steps += g_record_steps;

    g_pending_generation = g_generation + 1u;
    write32(g_transfer.data + 8, g_pending_generation);
    write16(g_transfer.data + 12, new_count);
    write16(g_transfer.data + 14, total_steps);
    write16(g_transfer.data + 16, new_desc_len);
    write16(g_transfer.data + 18, out_program);
    g_expected_len = MACRO_HEADER_SIZE + new_desc_len + out_program;
    write32(g_transfer.data + 20,
            crc32_ieee(g_transfer.data + MACRO_HEADER_SIZE,
                       g_expected_len - MACRO_HEADER_SIZE));
    g_expected_crc = crc32_ieee(g_transfer.data, g_expected_len);
    return validate_blob(g_transfer.data, g_expected_len) && persist_transfer();
}

static void recorder_finish_deferred(uint64_t now_us)
{
    if (!g_record_finalize_pending) return;
    g_record_finalize_pending = false;
    if (g_record_have_state) {
        uint32_t duration = (uint32_t)((now_us - g_record_last_us) / 1000ULL);
        if (duration == 0) duration = 1;
        if (!recorder_append_state(&g_record_previous, duration)) {
            led_status_set(LED_RED_BLINK_FAST);
            g_recorder_state = RECORDER_SAVED;
            g_recorder_deadline_us = now_us + 1500000ULL;
            return;
        }
    }
    macro_engine_stop_all();
    if (g_record_steps && recorder_build_blob()) {
        led_status_set(LED_GREEN_BLINK_TRIPLE);
        LOG_INF("[MACRO] Standalone recording saved: trigger=%u steps=%u bytes=%u\n",
                g_record_trigger, g_record_steps, g_record_len);
    } else {
        led_status_set(LED_RED_BLINK_FAST);
        LOG_WRN("[MACRO] Standalone recording save failed\n");
    }
    g_recorder_state = RECORDER_SAVED;
    g_recorder_deadline_us = now_us + 1500000ULL;
    publish_status();
}

static void management_mark_dirty(void)
{
    if (g_active_len < MACRO_HEADER_SIZE) return;
    write32(g_active_blob + 8, g_generation + 1u);
    write32(g_active_blob + 20,
            crc32_ieee(g_active_blob + MACRO_HEADER_SIZE,
                       g_active_len - MACRO_HEADER_SIZE));
    g_management_dirty = true;
    publish_status();
}

static void management_persist_deferred(void)
{
    if (!g_management_dirty || g_state == MACRO_STATE_RECEIVING) return;
    g_management_dirty = false;
    memcpy(g_transfer.data, g_active_blob, g_active_len);
    g_pending_generation = g_generation + 1u;
    write32(g_transfer.data + 8, g_pending_generation);
    write32(g_transfer.data + 20,
            crc32_ieee(g_transfer.data + MACRO_HEADER_SIZE,
                       g_active_len - MACRO_HEADER_SIZE));
    g_expected_len = g_active_len;
    g_expected_crc = crc32_ieee(g_transfer.data, g_expected_len);
    if (!persist_transfer()) {
        set_error(MACRO_ERROR_STORAGE);
        led_status_set(LED_RED_BLINK_FAST);
    } else {
        g_state = MACRO_STATE_READY;
        g_error = MACRO_ERROR_NONE;
        publish_status();
    }
}

static void management_exit(void)
{
    g_management_mode = false;
    g_management_wait_release = false;
    g_management_wait_trigger = false;
    led_status_unlock();
    led_status_set(g_management_restore_led);
    publish_status();
    LOG_INF("[MACRO] Management mode exited\n");
}

static void management_handle(uint32_t pressed)
{
    if (g_management_wait_release) {
        if (g_raw_mask == 0) g_management_wait_release = false;
        return;
    }
    if (pressed & (1u << REMAP_BTN_PS)) {
        macro_engine_stop_all();
        management_exit();
        return;
    }
    if (pressed & (1u << REMAP_BTN_MUTE)) {
        management_exit();
        return;
    }
    if (g_active_len < MACRO_HEADER_SIZE) return;
    if (g_management_wait_trigger) {
        uint32_t choices = pressed & REMAP_TARGET_MASK &
                           ~(1u << REMAP_BTN_L1) & ~(1u << REMAP_BTN_PS);
        if (choices) {
            uint8_t trigger = 0;
            while (trigger < REMAP_BTN_COUNT && !(choices & (1u << trigger))) trigger++;
            const uint16_t count = read16(g_active_blob + 12);
            bool any_enabled = false;
            for (uint16_t i = 0; i < count; ++i) {
                uint8_t *d = g_active_blob + MACRO_HEADER_SIZE + i * MACRO_DESC_SIZE;
                if (d[4] == blob_profile() && d[5] == trigger && (d[8] & 1u))
                    any_enabled = true;
            }
            for (uint16_t i = 0; i < count; ++i) {
                uint8_t *d = g_active_blob + MACRO_HEADER_SIZE + i * MACRO_DESC_SIZE;
                if (d[4] == blob_profile() && d[5] == trigger)
                    d[8] = any_enabled ? (d[8] & ~1u) : (d[8] | 1u);
            }
            g_management_wait_trigger = false;
            management_mark_dirty();
        }
        return;
    }
    if (pressed & (1u << REMAP_BTN_SQUARE)) {
        g_active_blob[7] ^= 0x02u;
        management_mark_dirty();
    } else if (pressed & (1u << REMAP_BTN_CROSS)) {
        g_active_blob[7] ^= 0x04u;
        management_mark_dirty();
    } else if (pressed & (1u << REMAP_BTN_TP_CLICK)) {
        g_active_blob[7] ^= 0x01u;
        management_mark_dirty();
    } else if (pressed & (1u << REMAP_BTN_L1)) {
        g_management_wait_trigger = true;
    } else {
        for (uint8_t profile = 0; profile < 4; ++profile) {
            const uint8_t dpad = REMAP_BTN_DPAD_UP + profile;
            if (pressed & (1u << dpad)) {
                g_active_blob[6] = profile;
                management_mark_dirty();
                break;
            }
        }
    }
}

static bool validate_program(const uint8_t *program, uint16_t length,
                             uint16_t expected_steps)
{
    uint16_t cursor = 0;
    uint16_t waits = 0;
    while (cursor < length) {
        const uint8_t op = program[cursor++];
        if (op == 0x01) {
            if (cursor + 3u > length ||
                (read24(program + cursor) & ~REMAP_TARGET_MASK) != 0) return false;
            cursor += 3;
        } else if (op == 0x02) {
            if (cursor + 6u > length) return false;
            cursor += 6;
        } else if (op == 0x03) {
            if (cursor + 10u > length) return false;
            for (uint8_t i = 0; i < 2; ++i) {
                const uint8_t *point = program + cursor + i * 5u;
                if (point[0] > 1u || read16(point + 1) > 1919u ||
                    read16(point + 3) > 1079u) return false;
            }
            cursor += 10;
        } else if (op == 0x04) {
            if (cursor + 4u > length ||
                (read16(program + cursor) == 0u &&
                 read16(program + cursor + 2) == 0u)) return false;
            cursor += 4;
            waits++;
        } else {
            return false;
        }
    }
    return cursor == length && waits == expected_steps;
}

static bool validate_blob(const uint8_t *blob, uint16_t length)
{
    if (!blob || length < MACRO_HEADER_SIZE || length > MACRO_MAX_BYTES ||
        read32(blob) != MACRO_BLOB_MAGIC || blob[4] != MACRO_FORMAT_VERSION ||
        blob[5] != 4u || blob[6] >= 4u) return false;
    const uint16_t count = read16(blob + 12);
    const uint16_t steps = read16(blob + 14);
    const uint16_t descriptors = read16(blob + 16);
    const uint16_t programs = read16(blob + 18);
    if (count > MACRO_MAX_COUNT || steps > MACRO_MAX_STEPS ||
        descriptors != count * MACRO_DESC_SIZE ||
        (uint32_t)MACRO_HEADER_SIZE + descriptors + programs != length ||
        crc32_ieee(blob + MACRO_HEADER_SIZE, length - MACRO_HEADER_SIZE) !=
            read32(blob + 20)) return false;
    const uint8_t *program_base = blob + MACRO_HEADER_SIZE + descriptors;
    for (uint16_t i = 0; i < count; ++i) {
        const uint8_t *d = blob + MACRO_HEADER_SIZE + i * MACRO_DESC_SIZE;
        const uint8_t profile = d[4], trigger = d[5], trigger_mode = d[6];
        const uint8_t playback = d[7];
        const uint16_t step_count = read16(d + 16);
        const uint16_t offset = read16(d + 18), program_len = read16(d + 20);
        if (profile >= 4u || trigger >= REMAP_BTN_COUNT || trigger_mode > 4u ||
            playback > 5u || (uint32_t)offset + program_len > programs ||
            !validate_program(program_base + offset, program_len, step_count))
            return false;
        if (trigger == REMAP_BTN_PS && trigger_mode == 2u &&
            read16(d + 10) >= 2000u) return false;
        if (playback == 5u && (read16(d + 14) < 1u || read16(d + 14) > 50u))
            return false;
    }
    return true;
}

static bool load_slot(const char *key, uint8_t slot)
{
    size_t saved = 0;
    memset(&g_transfer, 0, offsetof(macro_store_t, data));
    size_t got = ef_get_env_blob(key, &g_transfer, sizeof(g_transfer), &saved);
    if (got < offsetof(macro_store_t, data) || saved != got ||
        g_transfer.magic != MACRO_STORE_MAGIC ||
        g_transfer.length > MACRO_MAX_BYTES ||
        got != offsetof(macro_store_t, data) + g_transfer.length ||
        crc32_ieee(g_transfer.data, g_transfer.length) != g_transfer.crc32 ||
        !validate_blob(g_transfer.data, g_transfer.length)) return false;
    if (g_active_len == 0 || g_transfer.generation > g_generation) {
        memcpy(g_active_blob, g_transfer.data, g_transfer.length);
        g_active_len = g_transfer.length;
        g_generation = g_transfer.generation;
        g_active_slot = slot;
    }
    return true;
}

static bool persist_transfer(void)
{
    const char *key = g_active_slot == 1u ? MACRO_STORE_KEY_B : MACRO_STORE_KEY_A;
    const uint8_t new_slot = g_active_slot == 1u ? 2u : 1u;
    g_transfer.magic = MACRO_STORE_MAGIC;
    g_transfer.generation = g_pending_generation;
    g_transfer.length = g_expected_len;
    g_transfer.reserved = 0;
    g_transfer.crc32 = g_expected_crc;
    const size_t store_len = offsetof(macro_store_t, data) + g_expected_len;
    if (ef_set_env_blob(key, &g_transfer, store_len) != EF_NO_ERR) return false;
    size_t saved = 0;
    if (ef_get_env_blob(key, NULL, 0, &saved) != 0 || saved != store_len)
        return false;
    memcpy(g_active_blob, g_transfer.data, g_expected_len);
    g_active_len = g_expected_len;
    g_generation = g_pending_generation;
    g_active_slot = new_slot;
    return true;
}

static const uint8_t *descriptor_at(uint16_t index)
{
    if (g_active_len < MACRO_HEADER_SIZE || index >= read16(g_active_blob + 12))
        return NULL;
    return g_active_blob + MACRO_HEADER_SIZE + index * MACRO_DESC_SIZE;
}

void macro_engine_stop_all(void)
{
    memset(&g_playback, 0, sizeof(g_playback));
    publish_status();
}

static void start_macro(uint16_t index, bool held, uint64_t now_us)
{
    const uint8_t *d = descriptor_at(index);
    if (!d) return;
    memset(&g_playback, 0, sizeof(g_playback));
    g_playback.active = true;
    g_playback.descriptor_index = index;
    g_playback.cursor = read16(d + 18);
    g_playback.program_end = g_playback.cursor + read16(d + 20);
    g_playback.repeats_left = read16(d + 12);
    g_playback.trigger_held = held;
    g_playback.deadline_us = now_us;
    publish_status();
}

static bool trigger_macro(uint8_t trigger, uint8_t event, uint64_t interval_us,
                          bool held, uint64_t now_us)
{
    if (g_active_len < MACRO_HEADER_SIZE || !(blob_flags() & 0x01u)) return false;
    const uint16_t count = read16(g_active_blob + 12);
    for (uint16_t i = 0; i < count; ++i) {
        const uint8_t *d = descriptor_at(i);
        if (!d || d[4] != blob_profile() || d[5] != trigger || !(d[8] & 0x01u))
            continue;
        if ((d[7] <= 1u && !(blob_flags() & 0x02u)) ||
            (d[7] >= 2u && !(blob_flags() & 0x04u))) continue;
        const bool trigger_match = d[6] == event ||
                                   (event == 0u && d[6] == 4u);
        if (trigger_match &&
            (d[6] != 3u || interval_us <= (uint64_t)read16(d + 10) * 1000ULL)) {
            if (d[7] == 4u && g_playback.active &&
                g_playback.descriptor_index == i) macro_engine_stop_all();
            else start_macro(i, held, now_us);
            return true; /* one event selects one macro; no recursion/fan-out */
        }
    }
    return false;
}

void macro_engine_observe_raw(const uint8_t *payload, uint64_t now_us)
{
    g_previous_raw_mask = g_raw_mask;
    g_raw_mask = remap_physical_mask(payload);
    const uint32_t pressed = g_raw_mask & ~g_previous_raw_mask;
    const uint32_t released = g_previous_raw_mask & ~g_raw_mask;
    const uint32_t record_chord = (1u << REMAP_BTN_CREATE) |
                                  (1u << REMAP_BTN_OPTIONS);
    const uint32_t management_chord = (1u << REMAP_BTN_MUTE) |
                                      (1u << REMAP_BTN_OPTIONS);
    const bool chord_down = (g_raw_mask & record_chord) == record_chord;
    const bool management_chord_down =
        (g_raw_mask & management_chord) == management_chord;

    if (g_recorder_state == RECORDER_SAVED && now_us >= g_recorder_deadline_us) {
        g_recorder_state = RECORDER_IDLE;
        recorder_restore_led();
    }
    if (chord_down) {
        if (g_record_chord_press_us == 0) g_record_chord_press_us = now_us;
        if (!g_record_chord_latched &&
            now_us - g_record_chord_press_us >= 2000000ULL) {
            g_record_chord_latched = true;
            if (g_recorder_state == RECORDER_IDLE &&
                g_state != MACRO_STATE_RECEIVING) {
                g_record_restore_led = led_status_get();
                led_status_lock();
                led_status_set(LED_YELLOW_SOLID);
                g_recorder_state = RECORDER_SELECT_TRIGGER;
                macro_engine_stop_all();
                LOG_INF("[MACRO] Recorder armed; waiting for physical trigger\n");
            } else if (g_recorder_state == RECORDER_ACTIVE) {
                g_record_finalize_pending = true;
            }
        }
    } else {
        g_record_chord_press_us = 0;
        g_record_chord_latched = false;
    }

    if (!g_management_mode && g_recorder_state == RECORDER_IDLE &&
        management_chord_down && g_state != MACRO_STATE_RECEIVING) {
        if (g_management_chord_press_us == 0) g_management_chord_press_us = now_us;
        if (now_us - g_management_chord_press_us >= 2000000ULL) {
            g_management_mode = true;
            g_management_wait_release = true;
            g_management_wait_trigger = false;
            g_management_restore_led = led_status_get();
            led_status_lock();
            led_status_set(LED_BLUE_SOLID);
            macro_engine_stop_all();
            publish_status();
            LOG_INF("[MACRO] Management mode entered\n");
        }
    } else if (!management_chord_down) {
        g_management_chord_press_us = 0;
    }

    if (g_recorder_state == RECORDER_SELECT_TRIGGER) {
        uint32_t choices = pressed & ~record_chord & ~(1u << REMAP_BTN_PS);
        if (choices) {
            for (uint8_t id = 0; id < REMAP_BTN_COUNT; ++id) {
                if (choices & (1u << id)) {
                    g_record_trigger = id;
                    break;
                }
            }
            g_recorder_state = RECORDER_COUNTDOWN;
            g_recorder_deadline_us = now_us + 3000000ULL;
            led_status_set(LED_YELLOW_SOLID);
            LOG_INF("[MACRO] Recorder trigger=%u; 3 second countdown\n",
                    g_record_trigger);
        }
    } else if (g_recorder_state == RECORDER_COUNTDOWN &&
               now_us >= g_recorder_deadline_us) {
        g_recorder_state = RECORDER_ACTIVE;
        g_record_len = g_record_steps = 0;
        g_record_have_state = false;
        g_record_warned = false;
        led_status_set(LED_GREEN_SOLID);
        LOG_INF("[MACRO] Standalone recording started\n");
    }

    if (pressed & (1u << REMAP_BTN_PS)) {
        g_ps_press_us = now_us;
        g_ps_emergency_fired = false;
    }
    if ((g_raw_mask & (1u << REMAP_BTN_PS)) && !g_ps_emergency_fired &&
        now_us - g_ps_press_us >= 2000000ULL) {
        macro_engine_stop_all();
        if (g_recorder_state != RECORDER_IDLE &&
            g_recorder_state != RECORDER_SAVED) recorder_cancel();
        g_ps_emergency_fired = true;
        LOG_INF("[MACRO] Physical PS emergency stop\n");
    }
    if (released & (1u << REMAP_BTN_PS)) g_ps_emergency_fired = false;

    if (g_management_mode) {
        management_handle(pressed);
        return;
    }

    /* Recorder/management shortcuts and the physical PS emergency stop stay
     * available even with macros disabled. Avoid the 19-control trigger scan
     * on the normal high-rate forwarding path when there is nothing to run,
     * and never start playback while a recording workflow owns the input. */
    if ((g_recorder_state != RECORDER_IDLE &&
         g_recorder_state != RECORDER_SAVED) ||
        g_active_len < MACRO_HEADER_SIZE || !(blob_flags() & 0x01u)) return;

    for (uint8_t id = 0; id < REMAP_BTN_COUNT; ++id) {
        const uint32_t bit = 1u << id;
        if (pressed & bit) {
            const uint64_t since_release = now_us - g_last_release_us[id];
            g_press_us[id] = now_us;
            g_long_fired_mask &= ~bit;
            bool double_started = false;
            if (g_last_release_us[id] != 0 && since_release <= 600000ULL)
                double_started = trigger_macro(id, 3u, since_release, true, now_us);
            if (!double_started)
                (void)trigger_macro(id, 0u, 0u, true, now_us);
        }
        if ((g_raw_mask & bit) && !(g_long_fired_mask & bit)) {
            const uint16_t count = g_active_len >= MACRO_HEADER_SIZE ?
                                   read16(g_active_blob + 12) : 0;
            for (uint16_t i = 0; i < count; ++i) {
                const uint8_t *d = descriptor_at(i);
                if (d && d[4] == blob_profile() && d[5] == id && d[6] == 2u &&
                    now_us - g_press_us[id] >= (uint64_t)read16(d + 10) * 1000ULL) {
                    g_long_fired_mask |= bit;
                    start_macro(i, true, now_us);
                    break;
                }
            }
        }
        if (released & bit) {
            g_last_release_us[id] = now_us;
            (void)trigger_macro(id, 1u, 0u, false, now_us);
            if (g_playback.active) {
                const uint8_t *d = descriptor_at(g_playback.descriptor_index);
                if (d && d[5] == id &&
                    (d[6] == 4u || d[7] == 1u || d[7] == 3u))
                    macro_engine_stop_all();
            }
        }
    }
}

void macro_engine_record_mapped(const uint8_t *payload, uint64_t now_us)
{
    if (!payload || g_recorder_state != RECORDER_ACTIVE ||
        g_record_finalize_pending) return;
    const uint32_t record_chord = (1u << REMAP_BTN_CREATE) |
                                  (1u << REMAP_BTN_OPTIONS);
    if ((g_raw_mask & record_chord) == record_chord) return;
    recorded_state_t current;
    current.mask = remap_logical_mask(payload);
    memcpy(current.axes, payload, 6);
    for (uint8_t i = 0; i < 2; ++i) {
        const uint8_t *source = payload + 32u + i * 4u;
        uint8_t *dest = current.touches + i * 5u;
        dest[0] = (source[0] & 0x80u) ? 0u : 1u;
        const uint16_t x = source[1] | ((uint16_t)(source[2] & 0x0Fu) << 8);
        const uint16_t y = (source[2] >> 4) | ((uint16_t)source[3] << 4);
        write16(dest + 1, x);
        write16(dest + 3, y);
    }
    if (!g_record_have_state) {
        g_record_previous = current;
        g_record_last_us = now_us;
        g_record_have_state = true;
        return;
    }
    if (memcmp(&current, &g_record_previous, sizeof(current)) == 0) return;
    uint32_t duration = (uint32_t)((now_us - g_record_last_us) / 1000ULL);
    if (duration == 0) duration = 1;
    if (!recorder_append_state(&g_record_previous, duration)) {
        led_status_set(LED_RED_BLINK_FAST);
        g_record_finalize_pending = true;
        return;
    }
    g_record_previous = current;
    g_record_last_us = now_us;
    if (!g_record_warned && g_record_len >= MACRO_RECORD_WARN_BYTES) {
        g_record_warned = true;
        led_status_set(LED_RED_BLINK_SLOW);
    }
}

static bool playback_next_step(uint64_t now_us)
{
    const uint8_t *d = descriptor_at(g_playback.descriptor_index);
    if (!d) return false;
    const uint16_t desc_bytes = read16(g_active_blob + 16);
    const uint8_t *program = g_active_blob + MACRO_HEADER_SIZE + desc_bytes;
    while (g_playback.cursor < g_playback.program_end) {
        const uint8_t op = program[g_playback.cursor++];
        if (op == 0x01) {
            g_playback.digital_mask = read24(program + g_playback.cursor);
            g_playback.cursor += 3;
        } else if (op == 0x02) {
            memcpy(g_playback.axes, program + g_playback.cursor, 6);
            g_playback.axis_valid = 1;
            g_playback.cursor += 6;
        } else if (op == 0x03) {
            memcpy(g_playback.touches, program + g_playback.cursor, 10);
            g_playback.touch_valid = true;
            g_playback.cursor += 10;
        } else if (op == 0x04) {
            uint32_t duration = read16(program + g_playback.cursor);
            const uint16_t random_extra = read16(program + g_playback.cursor + 2);
            g_playback.cursor += 4;
            if (random_extra) {
                g_rng = g_rng * 1664525u + 1013904223u;
                duration += g_rng % ((uint32_t)random_extra + 1u);
            }
            g_playback.deadline_us = now_us + (uint64_t)duration * 1000ULL;
            return true;
        } else return false;
    }

    uint16_t gap_ms = read16(d + 14);
    bool restart = false;
    switch (d[7]) {
    case 0: restart = false; break;
    case 1: restart = g_playback.trigger_held; break;
    case 2:
        if (g_playback.repeats_left > 1u) {
            g_playback.repeats_left--;
            restart = true;
        }
        break;
    case 3: restart = g_playback.trigger_held; break;
    case 4: restart = true; break;
    case 5:
        if (g_playback.repeats_left == 0u || g_playback.repeats_left > 1u) {
            if (g_playback.repeats_left > 1u) g_playback.repeats_left--;
            restart = true;
            const uint16_t hz = read16(d + 14);
            gap_ms = hz ? (uint16_t)(1000u / hz) : 20u;
        }
        break;
    default: break;
    }
    if (!restart) return false;
    g_playback.cursor = read16(d + 18);
    g_playback.deadline_us = now_us + (uint64_t)gap_ms * 1000ULL;
    return true;
}

void macro_engine_apply(uint8_t *payload, uint64_t now_us)
{
    if (!payload) return;
    if (g_management_mode) {
        memset(payload, 0, 40);
        payload[0] = payload[1] = payload[2] = payload[3] = 0x7F;
        payload[7] = 8;
        payload[32] = payload[36] = 0x80;
        return;
    }
    if (!g_playback.active) return;
    if (now_us >= g_playback.deadline_us && !playback_next_step(now_us)) {
        macro_engine_stop_all();
        return;
    }
    const uint8_t *d = descriptor_at(g_playback.descriptor_index);
    uint32_t physical = remap_logical_mask(payload);
    if (d && !(d[8] & 0x02u)) physical &= ~(1u << d[5]);
    uint32_t merged = physical | g_playback.digital_mask;
    if (g_playback.axis_valid && g_playback.axes[4])
        merged |= 1u << REMAP_BTN_L2;
    if (g_playback.axis_valid && g_playback.axes[5])
        merged |= 1u << REMAP_BTN_R2;
    const uint8_t l2 = g_playback.axis_valid ? g_playback.axes[4] : payload[4];
    const uint8_t r2 = g_playback.axis_valid ? g_playback.axes[5] : payload[5];
    remap_write_logical_mask(payload, merged, l2, r2);
    if (g_playback.axis_valid) memcpy(payload, g_playback.axes, 4);
    if (g_playback.touch_valid) {
        for (uint8_t i = 0; i < 2; ++i) {
            const uint8_t *src = g_playback.touches + i * 5u;
            uint8_t *dst = payload + 32u + i * 4u;
            const uint16_t x = read16(src + 1), y = read16(src + 3);
            dst[0] = src[0] ? i : (uint8_t)(0x80u | i);
            dst[1] = (uint8_t)x;
            dst[2] = (uint8_t)((x >> 8) | ((y & 0x0Fu) << 4));
            dst[3] = (uint8_t)(y >> 4);
        }
    }
}

void macro_engine_on_disconnect(void)
{
    macro_engine_stop_all();
    g_raw_mask = g_previous_raw_mask = 0;
    g_ps_press_us = 0;
    g_ps_emergency_fired = false;
    if (g_management_mode) {
        g_management_mode = false;
        led_status_unlock();
    }
    if (g_recorder_state != RECORDER_IDLE) recorder_cancel();
}

int macro_engine_init(void)
{
    memset(g_active_blob, 0, sizeof(g_active_blob));
    memset(&g_transfer, 0, sizeof(g_transfer));
    memset(&g_playback, 0, sizeof(g_playback));
    memset(g_press_us, 0, sizeof(g_press_us));
    memset(g_last_release_us, 0, sizeof(g_last_release_us));
    g_active_len = 0;
    g_generation = 0;
    g_active_slot = 0;
    g_state = MACRO_STATE_IDLE;
    g_error = MACRO_ERROR_NONE;
    (void)load_slot(MACRO_STORE_KEY_A, 1);
    (void)load_slot(MACRO_STORE_KEY_B, 2);
    g_command_queue = xQueueCreate(MACRO_QUEUE_DEPTH, sizeof(macro_command_frame_t));
    if (!g_command_queue) return -1;
    g_state = g_active_len ? MACRO_STATE_READY : MACRO_STATE_IDLE;
    publish_status();
    LOG_INF("[MACRO] v3 store: generation=%lu bytes=%u slot=%u\n",
            (unsigned long)g_generation, g_active_len, g_active_slot);
    return 0;
}

bool macro_engine_enqueue_from_isr(const uint8_t *payload, uint32_t len)
{
    if (!g_command_queue || !payload || len != MACRO_REPORT_SIZE) return false;
    macro_command_frame_t frame;
    memcpy(frame.payload, payload, sizeof(frame.payload));
    BaseType_t wake = pdFALSE;
    if (xQueueSendFromISR(g_command_queue, &frame, &wake) != pdTRUE) {
        g_error = MACRO_ERROR_QUEUE_FULL;
        return false;
    }
    portYIELD_FROM_ISR(wake);
    return true;
}

void macro_engine_get_report(uint8_t out[MACRO_REPORT_SIZE])
{
    if (!out) return;
    taskENTER_CRITICAL();
    memcpy(out, g_status, MACRO_REPORT_SIZE);
    taskEXIT_CRITICAL();
}

static void handle_command(const uint8_t p[MACRO_REPORT_SIZE])
{
    if (p[0] != 'M' || p[1] != 'C' || p[2] != MACRO_FORMAT_VERSION) {
        set_error(MACRO_ERROR_FRAME);
        return;
    }
    const uint8_t command = p[3];
    if (command == MACRO_CMD_BEGIN) {
        const uint32_t expected_generation = read32(p + 4);
        const uint32_t new_generation = read32(p + 8);
        const uint16_t length = read16(p + 12);
        if (expected_generation != g_generation || new_generation <= g_generation) {
            set_error(MACRO_ERROR_STALE);
            return;
        }
        if (length < MACRO_HEADER_SIZE || length > MACRO_MAX_BYTES) {
            set_error(MACRO_ERROR_BOUNDS);
            return;
        }
        g_pending_generation = new_generation;
        g_expected_len = length;
        g_expected_crc = read32(p + 14);
        g_received = 0;
        memset(g_transfer.data, 0, length);
        g_error = MACRO_ERROR_NONE;
        g_state = MACRO_STATE_RECEIVING;
    } else if (command == MACRO_CMD_CHUNK) {
        const uint16_t offset = read16(p + 4);
        const uint8_t length = p[6];
        if (g_state != MACRO_STATE_RECEIVING || length > 56u ||
            offset != g_received || (uint32_t)offset + length > g_expected_len) {
            set_error(offset != g_received ? MACRO_ERROR_ORDER : MACRO_ERROR_BOUNDS);
            return;
        }
        memcpy(g_transfer.data + offset, p + 7, length);
        g_received += length;
    } else if (command == MACRO_CMD_COMMIT) {
        if (g_state != MACRO_STATE_RECEIVING || g_received != g_expected_len) {
            set_error(MACRO_ERROR_ORDER);
            return;
        }
        if (crc32_ieee(g_transfer.data, g_expected_len) != g_expected_crc) {
            set_error(MACRO_ERROR_CRC);
            return;
        }
        /* Generation belongs to the transactional store, and is mirrored into
         * the compiled header before its internal CRC is validated. */
        write32(g_transfer.data + 8, g_pending_generation);
        write32(g_transfer.data + 20,
                crc32_ieee(g_transfer.data + MACRO_HEADER_SIZE,
                           g_expected_len - MACRO_HEADER_SIZE));
        g_expected_crc = crc32_ieee(g_transfer.data, g_expected_len);
        if (!validate_blob(g_transfer.data, g_expected_len)) {
            set_error(MACRO_ERROR_FORMAT);
            return;
        }
        macro_engine_stop_all();
        if (!persist_transfer()) {
            set_error(MACRO_ERROR_STORAGE);
            return;
        }
        g_state = MACRO_STATE_READY;
        g_error = MACRO_ERROR_NONE;
        g_read_offset = 0;
    } else if (command == MACRO_CMD_ABORT) {
        g_state = g_active_len ? MACRO_STATE_READY : MACRO_STATE_IDLE;
        g_error = MACRO_ERROR_NONE;
        g_received = 0;
    } else if (command == MACRO_CMD_READ_SELECT) {
        const uint16_t offset = read16(p + 4);
        if (offset > g_active_len) {
            set_error(MACRO_ERROR_BOUNDS);
            return;
        }
        g_read_offset = offset;
    } else if (command == MACRO_CMD_CONTROL) {
        const uint32_t expected = read32(p + 6);
        if (g_active_len < MACRO_HEADER_SIZE || expected != g_generation ||
            p[5] >= 4u || (p[4] & ~0x07u)) {
            set_error(expected != g_generation ? MACRO_ERROR_STALE : MACRO_ERROR_FRAME);
            return;
        }
        memcpy(g_transfer.data, g_active_blob, g_active_len);
        g_transfer.data[6] = p[5];
        g_transfer.data[7] = p[4];
        g_pending_generation = g_generation + 1u;
        write32(g_transfer.data + 8, g_pending_generation);
        write32(g_transfer.data + 20,
                crc32_ieee(g_transfer.data + MACRO_HEADER_SIZE,
                           g_active_len - MACRO_HEADER_SIZE));
        g_expected_len = g_active_len;
        g_expected_crc = crc32_ieee(g_transfer.data, g_active_len);
        macro_engine_stop_all();
        if (!persist_transfer()) {
            set_error(MACRO_ERROR_STORAGE);
            return;
        }
        g_state = MACRO_STATE_READY;
        g_error = MACRO_ERROR_NONE;
    } else if (command == MACRO_CMD_STOP) {
        macro_engine_stop_all();
        g_error = MACRO_ERROR_NONE;
        g_state = g_active_len ? MACRO_STATE_READY : MACRO_STATE_IDLE;
    } else {
        set_error(MACRO_ERROR_FRAME);
        return;
    }
    publish_status();
}

void macro_engine_process_deferred(void)
{
    macro_command_frame_t frame;
    while (g_command_queue && xQueueReceive(g_command_queue, &frame, 0) == pdTRUE)
        handle_command(frame.payload);
    recorder_finish_deferred(bflb_mtimer_get_time_us());
    management_persist_deferred();
}
