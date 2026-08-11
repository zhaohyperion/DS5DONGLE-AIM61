#include "runtime_diag.h"

#include "audio.h"
#include "bt_hid_host.h"
#include "compiler/compiler_ld.h"
#include "debug_log.h"
#include "ds5_usb_audio.h"
#include "mm.h"
#include "ota_update.h"
#include "usb_gamepad.h"

#include "usbd_core.h"

#include <limits.h>
#include <string.h>

/* A complete snapshot is built in the inactive bank from task context.  EP0
 * copies only from the published bank, so it can never observe a partly
 * formatted page or a partly written CRC. */
static uint8_t diag_reports[2][RUNTIME_DIAG_PAGE_COUNT]
                           [RUNTIME_DIAG_REPORT_SIZE]
               __attribute__((aligned(4)));
static volatile uint8_t diag_report_index;
static volatile uint8_t diag_selected_page;

static volatile uint32_t diag_get_requests;
static volatile uint32_t diag_select_requests;
static volatile uint32_t diag_select_errors;

static uint32_t diag_sequence;
static uint64_t diag_last_publish_us;
static uint32_t diag_min_free_bytes;

extern uint8_t get_battery_level(void);
extern uint8_t get_battery_state(void);

enum diag_header_flags {
    DIAG_HEADER_VALID       = 1u << 0,
    DIAG_HEADER_COHERENT    = 1u << 1,
    DIAG_HEADER_PSRAM       = 1u << 2,
    DIAG_HEADER_OTA_ACTIVE  = 1u << 3,
};

enum diag_health_flags {
    DIAG_HEALTH_USB_CONFIGURED = 1u << 0,
    DIAG_HEALTH_BT_CONNECTED   = 1u << 1,
    DIAG_HEALTH_DSE            = 1u << 2,
    DIAG_HEALTH_SPEAKER        = 1u << 3,
    DIAG_HEALTH_MICROPHONE     = 1u << 4,
    DIAG_HEALTH_OTA_ACTIVE     = 1u << 5,
    DIAG_HEALTH_PSRAM          = 1u << 6,
    DIAG_HEALTH_USB_SUSPENDED  = 1u << 7,
};

enum diag_usb_flags {
    DIAG_USB_CONFIGURED = 1u << 0,
    DIAG_USB_SUSPENDED  = 1u << 1,
    DIAG_USB_MAINTENANCE = 1u << 2,
    DIAG_USB_DSE_MODE   = 1u << 3,
    DIAG_USB_INPUT_BUSY = 1u << 4,
    DIAG_USB_KEYBOARD   = 1u << 5,
};

enum diag_capabilities {
    DIAG_CAP_PAGED       = 1u << 0,
    DIAG_CAP_CRC32       = 1u << 1,
    DIAG_CAP_COHERENT    = 1u << 2,
    DIAG_CAP_MONOTONIC   = 1u << 3,
    DIAG_CAP_READ_ONLY   = 1u << 4,
    DIAG_CAP_SELECT_PAGE = 1u << 5,
};

static void write_le16(uint8_t *out, uint16_t value)
{
    out[0] = (uint8_t)value;
    out[1] = (uint8_t)(value >> 8);
}

static void write_le32(uint8_t *out, uint32_t value)
{
    out[0] = (uint8_t)value;
    out[1] = (uint8_t)(value >> 8);
    out[2] = (uint8_t)(value >> 16);
    out[3] = (uint8_t)(value >> 24);
}

static uint32_t read_le32(const uint8_t *in)
{
    return (uint32_t)in[0] |
           ((uint32_t)in[1] << 8) |
           ((uint32_t)in[2] << 16) |
           ((uint32_t)in[3] << 24);
}

static uint16_t saturate_us(uint32_t value, uint8_t bit, uint8_t *flags)
{
    if (value > UINT16_MAX) {
        *flags |= bit;
        return UINT16_MAX;
    }
    return (uint16_t)value;
}

static void finish_page(uint8_t *page, uint8_t page_index,
                        uint8_t data_len, uint8_t flags,
                        uint32_t sequence, uint32_t monotonic_ms)
{
    page[0] = RUNTIME_DIAG_MAGIC_0;
    page[1] = RUNTIME_DIAG_MAGIC_1;
    page[2] = RUNTIME_DIAG_PROTOCOL_VERSION;
    page[3] = RUNTIME_DIAG_HEADER_SIZE;
    page[4] = page_index;
    page[5] = RUNTIME_DIAG_PAGE_COUNT;
    page[6] = data_len;
    page[7] = flags;
    write_le32(page + 8, sequence);
    write_le32(page + 12, monotonic_ms);
    write_le32(page + RUNTIME_DIAG_CRC_OFFSET,
               ota_update_crc32(page, RUNTIME_DIAG_CRC_OFFSET));
}

static uint8_t common_header_flags(bool ota_active)
{
    uint8_t flags = DIAG_HEADER_VALID | DIAG_HEADER_COHERENT;
#ifdef CONFIG_PSRAM
    flags |= DIAG_HEADER_PSRAM;
#endif
    if (ota_active)
        flags |= DIAG_HEADER_OTA_ACTIVE;
    return flags;
}

void runtime_diag_init(void)
{
    memset(diag_reports, 0, sizeof(diag_reports));
    diag_report_index = 0;
    diag_selected_page = RUNTIME_DIAG_PAGE_IDENTITY;
    diag_get_requests = 0;
    diag_select_requests = 0;
    diag_select_errors = 0;
    diag_sequence = 0;
    diag_last_publish_us = 0;
    diag_min_free_bytes = UINT32_MAX;

    /* A host probing immediately after enumeration still receives a valid,
     * CRC-protected response.  The first usb_task pass replaces it with live
     * values and sequence 1. */
    for (uint8_t bank = 0; bank < 2; bank++) {
        for (uint8_t page = 0; page < RUNTIME_DIAG_PAGE_COUNT; page++) {
            finish_page(diag_reports[bank][page], page, 0,
                        DIAG_HEADER_VALID | DIAG_HEADER_COHERENT, 0, 0);
        }
    }
}

void runtime_diag_task_update(uint64_t monotonic_us)
{
    const uint64_t interval_us =
        (uint64_t)RUNTIME_DIAG_PUBLISH_INTERVAL_MS * 1000u;
    if (diag_sequence != 0 &&
        (monotonic_us - diag_last_publish_us) < interval_us) {
        return;
    }
    diag_last_publish_us = monotonic_us;

    struct usb_gamepad_runtime_stats usb_stats;
    struct usb_gamepad_link_status link;
    struct bt_hid_tx_stats bt_stats;
    audio_runtime_diag_stats_t audio_stats;
    usb_audio_stats_t uac_stats;
    uint8_t ota_status[OTA_REPORT_PAYLOAD_SIZE];

    usb_gamepad_get_runtime_stats(&usb_stats);
    usb_gamepad_get_link_status(&link);
    bt_hid_host_get_tx_stats(&bt_stats);
    audio_get_runtime_diag_stats(&audio_stats);
    usb_audio_get_stats(&uac_stats);
    ota_update_get_status_report(ota_status);

    const bool ota_active = ota_update_maintenance_active();
    const bool speaker_active = usb_audio_is_active();
    const bool mic_active = usb_audio_mic_is_active();
    const enum bt_hid_host_state bt_state = bt_hid_host_get_state();
    const bool bt_connected = bt_state == BT_HID_STATE_CONNECTED;
    const bool dse = bt_hid_host_is_dse();
    const uint32_t monotonic_ms = (uint32_t)(monotonic_us / 1000u);
    const uint32_t sequence = ++diag_sequence;
    const uint8_t header_flags = common_header_flags(ota_active);
    uint8_t next = diag_report_index ^ 1u;
    uint8_t (*pages)[RUNTIME_DIAG_REPORT_SIZE] = diag_reports[next];

    memset(pages, 0, sizeof(diag_reports[next]));

    /* Page 0: identity and current health. */
    uint8_t *data = pages[RUNTIME_DIAG_PAGE_IDENTITY] +
                    RUNTIME_DIAG_DATA_OFFSET;
    uint8_t health = 0;
    uint8_t usb_flags = 0;
    int8_t rssi = bt_hid_host_get_cached_rssi();
    if (!bt_connected || rssi >= 0)
        rssi = INT8_MAX; /* never expose a stale RSSI after disconnect */

    if (link.configured) {
        health |= DIAG_HEALTH_USB_CONFIGURED;
        usb_flags |= DIAG_USB_CONFIGURED;
    }
    if (link.suspended) {
        health |= DIAG_HEALTH_USB_SUSPENDED;
        usb_flags |= DIAG_USB_SUSPENDED;
    }
    if (link.maintenance)
        usb_flags |= DIAG_USB_MAINTENANCE;
    if (link.dse_mode)
        usb_flags |= DIAG_USB_DSE_MODE;
    if (link.input_busy)
        usb_flags |= DIAG_USB_INPUT_BUSY;
    if (link.keyboard_registered)
        usb_flags |= DIAG_USB_KEYBOARD;
    if (bt_connected)
        health |= DIAG_HEALTH_BT_CONNECTED;
    if (dse)
        health |= DIAG_HEALTH_DSE;
    if (speaker_active)
        health |= DIAG_HEALTH_SPEAKER;
    if (mic_active)
        health |= DIAG_HEALTH_MICROPHONE;
    if (ota_active)
        health |= DIAG_HEALTH_OTA_ACTIVE;
#ifdef CONFIG_PSRAM
    health |= DIAG_HEALTH_PSRAM;
#endif

    write_le32(data + 0, monotonic_ms);
    data[4] = (uint8_t)bt_state;
    data[5] = health;
    data[6] = (uint8_t)rssi;
    data[7] = get_battery_level();
    data[8] = get_battery_state();
    data[9] = OTA_CURRENT_BOARD;
    data[10] = usbd_get_port_speed(0);
    data[11] = OTA_CURRENT_USB_SPEED;
    data[12] = LOG_LEVEL;
    data[13] = APP_VER_X;
    data[14] = APP_VER_Y;
    data[15] = APP_VER_Z;
    data[16] = bt_hid_host_get_bonded_count();
    data[17] = data[16] != 0 ? bt_hid_host_get_active_idx() : 0xFFu;
    data[18] = usb_flags;
    data[19] = bt_hid_host_is_switching() ? 1u : 0u;
    data[20] = DIAG_CAP_PAGED | DIAG_CAP_CRC32 | DIAG_CAP_COHERENT |
               DIAG_CAP_MONOTONIC | DIAG_CAP_READ_ONLY |
               DIAG_CAP_SELECT_PAGE;
    write_le32(data + 21, diag_get_requests);
    write_le32(data + 25, diag_select_requests);
    write_le32(data + 29, diag_select_errors);
    write_le32(data + 33, RUNTIME_DIAG_PUBLISH_INTERVAL_MS);
    finish_page(pages[RUNTIME_DIAG_PAGE_IDENTITY],
                RUNTIME_DIAG_PAGE_IDENTITY, 37, header_flags,
                sequence, monotonic_ms);

    /* Page 1: USB and Bluetooth traffic.  All counters are cumulative u32
     * values and therefore wrap naturally modulo 2^32. */
    data = pages[RUNTIME_DIAG_PAGE_USB_BT] + RUNTIME_DIAG_DATA_OFFSET;
    uint32_t bt_output_completed = bt_stats.game_completed +
                                   bt_stats.audio_completed +
                                   bt_stats.control_completed;
    write_le32(data + 0, usb_stats.transfers_completed);
    write_le32(data + 4, bt_stats.input_reports);
    write_le32(data + 8, bt_output_completed);
    write_le32(data + 12, usb_stats.input_updates);
    write_le32(data + 16, usb_stats.input_coalesced);
    write_le32(data + 20, usb_stats.transfers_started);
    write_le32(data + 24, usb_stats.start_errors);
    write_le32(data + 28, bt_stats.game_enqueued);
    write_le32(data + 32, bt_stats.game_coalesced);
    write_le32(data + 36, bt_stats.game_completed);
    data[40] = (bt_stats.game_pending ? 1u : 0u) |
               (bt_stats.in_flight ? 2u : 0u);
    data[41] = bt_stats.audio_queued;
    data[42] = bt_stats.control_queued;
    finish_page(pages[RUNTIME_DIAG_PAGE_USB_BT],
                RUNTIME_DIAG_PAGE_USB_BT, 43, header_flags,
                sequence, monotonic_ms);

    /* Page 2: loss/backpressure pressure points.  Offset 0 is an aggregate
     * trend counter for the UI; individual cumulative sources follow. */
    data = pages[RUNTIME_DIAG_PAGE_PRESSURE] + RUNTIME_DIAG_DATA_OFFSET;
    uint32_t pressure = usb_stats.start_errors +
                        bt_stats.audio_backpressure +
                        bt_stats.audio_dropped_stale +
                        bt_stats.control_backpressure +
                        bt_stats.alloc_failures +
                        bt_stats.send_failures +
                        bt_stats.stale_completions +
                        uac_stats.pcm_blocks_dropped +
                        uac_stats.pcm_pool_starvations +
                        uac_stats.pcm_queue_overruns +
                        uac_stats.mic_ring_overruns +
                        uac_stats.mic_ring_underruns;
    write_le32(data + 0, pressure);
    write_le32(data + 4, usb_stats.start_errors);
    write_le32(data + 8, bt_stats.audio_backpressure);
    write_le32(data + 12, bt_stats.audio_dropped_stale);
    write_le32(data + 16, bt_stats.control_backpressure);
    write_le32(data + 20, bt_stats.alloc_failures);
    write_le32(data + 24, bt_stats.send_failures);
    write_le32(data + 28, bt_stats.stale_completions);
    write_le32(data + 32, uac_stats.pcm_blocks_dropped);
    write_le32(data + 36, uac_stats.pcm_pool_starvations);
    data[40] = bt_stats.audio_high_watermark;
    data[41] = bt_stats.control_high_watermark;
    data[42] = audio_stats.encode_errors != 0 ? 1u : 0u;
    finish_page(pages[RUNTIME_DIAG_PAGE_PRESSURE],
                RUNTIME_DIAG_PAGE_PRESSURE, 43, header_flags,
                sequence, monotonic_ms);

    /* Page 3: audio processing health.  Microsecond timing fields are u16
     * saturated at 65535; byte 40 records which timing families saturated. */
    data = pages[RUNTIME_DIAG_PAGE_AUDIO_TIMING] +
           RUNTIME_DIAG_DATA_OFFSET;
    uint8_t timing_flags = 0;
    write_le32(data + 0, audio_stats.frames_processed);
    write_le32(data + 4, audio_stats.pairs_submitted);
    write_le32(data + 8, audio_stats.pairs_rejected);
    write_le32(data + 12, audio_stats.discontinuities);
    write_le32(data + 16, audio_stats.encode_errors);
    write_le32(data + 20, audio_stats.pipeline_resets);
    write_le32(data + 24, audio_stats.deadline_misses);
    write_le16(data + 28,
               saturate_us(audio_stats.block_us_p99, 1u << 0, &timing_flags));
    write_le16(data + 30,
               saturate_us(audio_stats.block_us_max, 1u << 0, &timing_flags));
    write_le16(data + 32,
               saturate_us(audio_stats.resample_us_p99, 1u << 1,
                           &timing_flags));
    write_le16(data + 34,
               saturate_us(audio_stats.resample_us_max, 1u << 1,
                           &timing_flags));
    write_le16(data + 36,
               saturate_us(audio_stats.encode_us_p99, 1u << 2,
                           &timing_flags));
    write_le16(data + 38,
               saturate_us(audio_stats.encode_us_max, 1u << 2,
                           &timing_flags));
    data[40] = timing_flags;
    finish_page(pages[RUNTIME_DIAG_PAGE_AUDIO_TIMING],
                RUNTIME_DIAG_PAGE_AUDIO_TIMING, 41, header_flags,
                sequence, monotonic_ms);

    /* Page 4: USB audio and microphone pipeline. */
    data = pages[RUNTIME_DIAG_PAGE_AUDIO_PIPELINE] +
           RUNTIME_DIAG_DATA_OFFSET;
    write_le32(data + 0, uac_stats.pcm_blocks_queued);
    write_le32(data + 4, audio_stats.pairs_submitted);
    write_le32(data + 8, uac_stats.mic_ring_underruns);
    write_le32(data + 12, uac_stats.mic_ring_overruns);
    write_le32(data + 16, uac_stats.pcm_blocks_dropped);
    write_le32(data + 20, uac_stats.pcm_pool_starvations);
    write_le32(data + 24, uac_stats.pcm_queue_overruns);
    write_le32(data + 28, uac_stats.pcm_stale_blocks);
    write_le32(data + 32, uac_stats.pcm_ready_high_water);
    write_le32(data + 36, uac_stats.mic_ep_write_errors);
    data[40] = (speaker_active ? 1u : 0u) |
               (mic_active ? 2u : 0u);
    finish_page(pages[RUNTIME_DIAG_PAGE_AUDIO_PIPELINE],
                RUNTIME_DIAG_PAGE_AUDIO_PIPELINE, 41, header_flags,
                sequence, monotonic_ms);

    /* Page 5: OTA progress and the SDK memory manager, sampled from task
     * context.  External PSRAM is intentionally not registered as a heap. */
    data = pages[RUNTIME_DIAG_PAGE_OTA_MEMORY] +
           RUNTIME_DIAG_DATA_OFFSET;
    const bool ota_valid = ota_status[0] == OTA_FRAME_MAGIC_0 &&
                           ota_status[1] == OTA_FRAME_MAGIC_1 &&
                           ota_status[2] == OTA_PROTOCOL_VERSION &&
                           read_le32(ota_status + OTA_REPORT_CRC_OFFSET) ==
                               ota_update_crc32(ota_status,
                                                OTA_REPORT_CRC_OFFSET);
    const uint8_t *ota_data = ota_status + 13;
    const uint32_t free_bytes = (uint32_t)kfree_size(0);
    if (free_bytes < diag_min_free_bytes)
        diag_min_free_bytes = free_bytes;
    write_le32(data + 0, free_bytes);
    write_le32(data + 4, diag_min_free_bytes);
    data[8] = ota_valid ? ota_data[OTA_STATUS_STATE_OFFSET] : 0xFFu;
    data[9] = ota_valid ? ota_data[OTA_STATUS_ERROR_OFFSET] : 0xFFu;
    data[10] = ota_valid ? ota_data[OTA_STATUS_LAST_REQUEST] : 0xFFu;
    data[11] = ota_valid ? ota_data[OTA_STATUS_FLAGS_OFFSET] : 0u;
    write_le32(data + 12, ota_valid ? read_le32(ota_status + 4) : 0u);
    write_le32(data + 16, ota_valid ? read_le32(ota_status + 8) : 0u);
    write_le32(data + 20,
               ota_valid ? read_le32(ota_data +
                                      OTA_STATUS_COMMITTED_OFFSET) : 0u);
    write_le32(data + 24,
               ota_valid ? read_le32(ota_data + OTA_STATUS_TOTAL_OFFSET) : 0u);
    write_le32(data + 28,
               ota_valid ? read_le32(ota_data +
                                      OTA_STATUS_MAX_FILE_OFFSET) : 0u);
    write_le32(data + 32,
               ota_valid ? read_le32(ota_data +
                                      OTA_STATUS_CAPABILITIES) : 0u);
#ifdef CONFIG_PSRAM
    write_le32(data + 36, 4u * 1024u * 1024u);
#else
    write_le32(data + 36, 0u);
#endif
    data[40] = ota_valid ? ota_data[OTA_STATUS_ACTIVE_SLOT] : 0xFFu;
    data[41] = ota_valid ? ota_data[OTA_STATUS_TRIAL_RETRY] : 0u;
    data[42] = 1u | /* internal SDK memory-manager values valid */
#ifdef CONFIG_PSRAM
               2u | 4u | /* PSRAM present and excluded from heap */
#endif
               (ota_valid ? 8u : 0u);
    finish_page(pages[RUNTIME_DIAG_PAGE_OTA_MEMORY],
                RUNTIME_DIAG_PAGE_OTA_MEMORY, 43, header_flags,
                sequence, monotonic_ms);

    __asm volatile("fence rw, rw" ::: "memory");
    diag_report_index = next;
}

ATTR_TCM_SECTION
bool runtime_diag_select_page_from_isr(const uint8_t *payload, uint32_t len)
{
    diag_select_requests++;
    if (!payload || len < RUNTIME_DIAG_SELECT_SIZE ||
        payload[0] != RUNTIME_DIAG_SELECT_PAGE ||
        payload[1] != RUNTIME_DIAG_PROTOCOL_VERSION ||
        payload[2] >= RUNTIME_DIAG_PAGE_COUNT) {
        diag_select_errors++;
        return false;
    }

    diag_selected_page = payload[2];
    return true;
}

ATTR_TCM_SECTION
void runtime_diag_get_selected_report(
    uint8_t out[RUNTIME_DIAG_REPORT_SIZE])
{
    if (!out)
        return;

    uint8_t page = diag_selected_page;
    if (page >= RUNTIME_DIAG_PAGE_COUNT)
        page = RUNTIME_DIAG_PAGE_IDENTITY;
    uint8_t index = diag_report_index;
    __asm volatile("fence r, r" ::: "memory");
    memcpy(out, diag_reports[index][page], RUNTIME_DIAG_REPORT_SIZE);
    diag_get_requests++;
}
