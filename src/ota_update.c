#include "ota_update.h"

#include "compiler/compiler_ld.h"
#include "debug_log.h"

#include "FreeRTOS.h"
#include "queue.h"
#include "task.h"

#include "bflb_boot2.h"
#include "bflb_wdg.h"
#include "bl_sys.h"
#include "ota.h"
#include "partition.h"

#include "mbedtls/ecdsa.h"
#include "mbedtls/ecp.h"
#include "mbedtls/sha256.h"
#include "mbedtls/version.h"

#include <string.h>

#if defined(__has_include)
#if __has_include("ota_release_public_key.h")
#include "ota_release_public_key.h"
#endif
#endif

#ifndef DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED
#define DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED 0
#endif

#ifndef DS5_OTA_RELEASE_PUBLIC_KEY_INITIALIZER
#define DS5_OTA_RELEASE_PUBLIC_KEY_INITIALIZER { 0 }
#endif

#ifndef DS5_OTA_ALLOW_UNSIGNED_DEV
#define DS5_OTA_ALLOW_UNSIGNED_DEV 0
#endif

#define OTA_EVENT_QUEUE_DEPTH       32u
#define OTA_TRIAL_RETRY_BYTE        0xF2u
#define OTA_TRIAL_WDG_SECONDS       15u
#define OTA_TRIAL_FEED_LIMIT_MS     10000u
#define OTA_TRIAL_HEALTH_HOLD_MS    3000u
#define OTA_SESSION_IDLE_TIMEOUT_MS 60000u

#if MBEDTLS_VERSION_MAJOR >= 3
#define OTA_MBEDTLS_FIELD(name) MBEDTLS_PRIVATE(name)
#else
#define OTA_MBEDTLS_FIELD(name) name
#endif

enum ota_event_kind {
    OTA_EVENT_DATA = 1,
    OTA_EVENT_CONTROL = 2,
};

typedef struct {
    uint8_t kind;
    uint8_t payload[OTA_REPORT_PAYLOAD_SIZE];
} ota_event_t;

typedef struct {
    ota_handle_t sdk;
    uint32_t session;
    uint32_t accepted_offset;
    uint32_t committed_offset;
    uint32_t total_size;
    uint32_t body_size;
    uint32_t max_image_size;
    uint8_t expected_body_sha256[32];
    uint8_t semver[3];
    uint8_t begin_flags;
    uint8_t signature[OTA_AUTH_SIGNATURE_SIZE];
    uint8_t auth_received;
    uint8_t slice[OTA_SDK_SLICE_SIZE] __attribute__((aligned(32)));
    uint32_t slice_len;
    uint8_t state;
    uint8_t error;
    uint8_t last_command;
    uint8_t active_slot;
    uint8_t trial_retry_byte;
    bool auth_required;
    bool auth_verified;
    bool header_checked;
    bool reboot_pending;
} ota_context_t;

static ota_context_t ota_ctx;
static QueueHandle_t ota_event_queue;
static volatile bool ota_queue_overflow;
static volatile bool ota_maintenance;
static ota_maintenance_hook_t maintenance_hook;

/* GET_REPORT may run while the OTA task is publishing a new response. */
static uint8_t ota_status_reports[2][OTA_REPORT_PAYLOAD_SIZE]
               __attribute__((aligned(4)));
static volatile uint8_t ota_status_index;

static const uint8_t ota_release_public_key[64] =
    DS5_OTA_RELEASE_PUBLIC_KEY_INITIALIZER;
static bool release_key_ready;

static struct bflb_device_s *trial_wdg;
static bool trial_boot;
static bool trial_watchdog_started;
static volatile bool runtime_healthy;
static volatile TickType_t runtime_healthy_tick;
static TickType_t trial_task_start_tick;
static TickType_t trial_last_feed_tick;
static TickType_t reboot_at_tick;
static TickType_t ota_last_activity_tick;

static uint32_t read_le32(const uint8_t *p)
{
    return (uint32_t)p[0] |
           ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static void write_le32(uint8_t *p, uint32_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 16);
    p[3] = (uint8_t)(value >> 24);
}

uint32_t ota_update_crc32(const uint8_t *data, uint32_t len)
{
    uint32_t crc = 0xFFFFFFFFu;

    if (!data)
        return 0;

    while (len--) {
        crc ^= *data++;
        for (uint8_t bit = 0; bit < 8; bit++)
            crc = (crc >> 1) ^
                  (0xEDB88320u & (uint32_t)-(int32_t)(crc & 1u));
    }
    return ~crc;
}

static bool bytes_are_zero(const uint8_t *data, uint32_t len)
{
    while (len--) {
        if (*data++ != 0)
            return false;
    }
    return true;
}

static uint8_t decimal_digits_u8(uint8_t value)
{
    if (value >= 100u)
        return 3u;
    if (value >= 10u)
        return 2u;
    return 1u;
}

static bool semver_fits_ota_header(const uint8_t semver[3])
{
    /* The SDK reserves at most 15 printable bytes in ver_software so the
     * 16-byte field remains NUL terminated.  "EVENT_V" consumes seven. */
    uint8_t text_len = decimal_digits_u8(semver[0]) +
                       decimal_digits_u8(semver[1]) +
                       decimal_digits_u8(semver[2]) + 2u;
    return text_len <= 8u;
}

static bool stable_signature_required(void)
{
    return DS5_OTA_ALLOW_UNSIGNED_DEV == 0;
}

static uint32_t current_capabilities(void)
{
    uint32_t capabilities = OTA_CAP_AB_SLOTS |
                            OTA_CAP_SHA256 |
                            OTA_CAP_TRIAL_BOOT |
                            OTA_CAP_MAINTENANCE |
                            OTA_CAP_RAW_OTA |
                            OTA_CAP_FRAME_CRC32 |
                            OTA_CAP_DELAYED_REBOOT;

    if (stable_signature_required())
        capabilities |= OTA_CAP_SIGNATURE_REQUIRED;
    if (release_key_ready)
        capabilities |= OTA_CAP_KEY_CONFIGURED;
    return capabilities;
}

static int load_release_public_key(mbedtls_ecp_group *group,
                                   mbedtls_ecp_point *point)
{
    int rc = mbedtls_ecp_group_load(group, MBEDTLS_ECP_DP_SECP256R1);
    if (rc != 0)
        return rc;

    rc = mbedtls_mpi_read_binary(&point->OTA_MBEDTLS_FIELD(X),
                                 ota_release_public_key, 32);
    if (rc == 0)
        rc = mbedtls_mpi_read_binary(&point->OTA_MBEDTLS_FIELD(Y),
                                     ota_release_public_key + 32, 32);
    if (rc == 0)
        rc = mbedtls_mpi_lset(&point->OTA_MBEDTLS_FIELD(Z), 1);
    if (rc == 0)
        rc = mbedtls_ecp_check_pubkey(group, point);
    return rc;
}

static bool release_public_key_is_valid(void)
{
#if DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED
    mbedtls_ecp_group group;
    mbedtls_ecp_point point;
    int rc;

    if (bytes_are_zero(ota_release_public_key,
                       sizeof(ota_release_public_key))) {
        return false;
    }

    mbedtls_ecp_group_init(&group);
    mbedtls_ecp_point_init(&point);
    rc = load_release_public_key(&group, &point);
    mbedtls_ecp_point_free(&point);
    mbedtls_ecp_group_free(&group);
    return rc == 0;
#else
    return false;
#endif
}

static bool verify_signature(void)
{
    static const uint8_t domain[] = "DS5DONGLE-OTA-V1";
    static const uint8_t p256_order[32] = {
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84,
        0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
    };
    static const uint8_t p256_half_order[32] = {
        0x7f, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00,
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xde, 0x73, 0x7d, 0x56, 0xd3, 0x8b, 0xcf, 0x42,
        0x79, 0xdc, 0xe5, 0x61, 0x7e, 0x31, 0x92, 0xa8,
    };
    uint8_t canonical[OTA_SIGN_CANONICAL_SIZE];
    uint8_t digest[32];
    mbedtls_ecp_group group;
    mbedtls_ecp_point point;
    mbedtls_mpi r;
    mbedtls_mpi s;
    uint32_t offset = 0;
    int rc;

    if (!release_key_ready || sizeof(domain) - 1u != 16u)
        return false;

    /* Reject invalid and non-canonical scalars before Mbed TLS verification.
     * Low-S removes ECDSA's (r, n-s) signature malleability. */
    if (bytes_are_zero(ota_ctx.signature, 32) ||
        bytes_are_zero(ota_ctx.signature + 32, 32) ||
        memcmp(ota_ctx.signature, p256_order, 32) >= 0 ||
        memcmp(ota_ctx.signature + 32, p256_order, 32) >= 0 ||
        memcmp(ota_ctx.signature + 32, p256_half_order, 32) > 0) {
        return false;
    }

    memcpy(canonical + offset, domain, sizeof(domain) - 1u);
    offset += sizeof(domain) - 1u;
    canonical[offset++] = OTA_CURRENT_BOARD;
    canonical[offset++] = OTA_CURRENT_USB_SPEED;
    memcpy(canonical + offset, ota_ctx.semver, sizeof(ota_ctx.semver));
    offset += sizeof(ota_ctx.semver);
    write_le32(canonical + offset, ota_ctx.body_size);
    offset += sizeof(uint32_t);
    memcpy(canonical + offset, ota_ctx.expected_body_sha256,
           sizeof(ota_ctx.expected_body_sha256));
    offset += sizeof(ota_ctx.expected_body_sha256);
    if (offset != sizeof(canonical))
        return false;

#if MBEDTLS_VERSION_MAJOR >= 3
    if (mbedtls_sha256(canonical, sizeof(canonical), digest, 0) != 0)
        return false;
#else
    mbedtls_sha256(canonical, sizeof(canonical), digest, 0);
#endif

    mbedtls_ecp_group_init(&group);
    mbedtls_ecp_point_init(&point);
    mbedtls_mpi_init(&r);
    mbedtls_mpi_init(&s);

    rc = load_release_public_key(&group, &point);
    if (rc == 0)
        rc = mbedtls_mpi_read_binary(&r, ota_ctx.signature, 32);
    if (rc == 0)
        rc = mbedtls_mpi_read_binary(&s, ota_ctx.signature + 32, 32);
    if (rc == 0)
        rc = mbedtls_ecdsa_verify(&group, digest, sizeof(digest),
                                  &point, &r, &s);

    mbedtls_mpi_free(&s);
    mbedtls_mpi_free(&r);
    mbedtls_ecp_point_free(&point);
    mbedtls_ecp_group_free(&group);
    return rc == 0;
}

static void refresh_partition_info(void)
{
    pt_table_stuff_config *table = bflb_boot2_get_pt_config();
    pt_table_entry_config entry;

    ota_ctx.max_image_size = 0;
    ota_ctx.active_slot = 0xFF;
    ota_ctx.trial_retry_byte = 0;

    if (!table ||
        pt_table_get_active_entries_by_name(table, (uint8_t *)"FW", &entry) !=
            PT_ERROR_SUCCESS) {
        return;
    }

    ota_ctx.active_slot = entry.active_index & 1u;
    ota_ctx.trial_retry_byte = (uint8_t)(entry.age >> 24);
    /* Always advertise the smaller FW slot.  Slot A is larger than slot B on
     * the 4 MiB M61 layout; accepting an A-sized image while running from B
     * would make the next A/B update impossible to place back into B. */
    uint32_t body_max = entry.max_len[0];
    if (entry.max_len[1] < body_max)
        body_max = entry.max_len[1];
    if (body_max > 0 && body_max <= UINT32_MAX - OTA_BOUFFALO_HEADER_SIZE)
        ota_ctx.max_image_size = body_max + OTA_BOUFFALO_HEADER_SIZE;
}

static void publish_status(void)
{
    uint8_t next = ota_status_index ^ 1u;
    uint8_t *out = ota_status_reports[next];
    uint8_t *data = out + 13;
    uint8_t flags = 0;

    memset(out, 0, OTA_REPORT_PAYLOAD_SIZE);
    out[0] = OTA_FRAME_MAGIC_0;
    out[1] = OTA_FRAME_MAGIC_1;
    out[2] = OTA_PROTOCOL_VERSION;
    out[3] = ota_ctx.error == OTA_ERROR_OK ? OTA_CTRL_ACK : OTA_CTRL_ERROR;
    write_le32(out + 4, ota_ctx.session);
    write_le32(out + 8, ota_ctx.accepted_offset);
    out[12] = OTA_STATUS_DATA_SIZE;

    if (ota_maintenance)
        flags |= OTA_STATUS_MAINTENANCE;
    if (ota_ctx.reboot_pending)
        flags |= OTA_STATUS_REBOOT_WAIT;
    if (trial_boot)
        flags |= OTA_STATUS_TRIAL_BOOT;

    data[OTA_STATUS_STATE_OFFSET] = ota_ctx.state;
    data[OTA_STATUS_ERROR_OFFSET] = ota_ctx.error;
    data[OTA_STATUS_LAST_REQUEST] = ota_ctx.last_command;
    data[OTA_STATUS_FLAGS_OFFSET] = flags;
    write_le32(data + OTA_STATUS_COMMITTED_OFFSET,
               ota_ctx.committed_offset);
    write_le32(data + OTA_STATUS_TOTAL_OFFSET, ota_ctx.total_size);
    write_le32(data + OTA_STATUS_MAX_FILE_OFFSET, ota_ctx.max_image_size);
    write_le32(data + OTA_STATUS_CAPABILITIES, current_capabilities());
    data[OTA_STATUS_BOARD_OFFSET] = OTA_CURRENT_BOARD;
    data[OTA_STATUS_SPEED_OFFSET] = OTA_CURRENT_USB_SPEED;
    data[OTA_STATUS_FORMAT_OFFSET] = OTA_FORMAT_RAW_OTA;
    data[OTA_STATUS_MAX_DATA_OFFSET] = OTA_DATA_BYTES_MAX;
    data[OTA_STATUS_WINDOW_OFFSET] = OTA_WINDOW_FRAMES;
    data[OTA_STATUS_ACTIVE_SLOT] = ota_ctx.active_slot;
    data[OTA_STATUS_TRIAL_RETRY] = ota_ctx.trial_retry_byte;
    data[OTA_STATUS_REBOOT_DELAY] = OTA_REBOOT_DELAY_100MS;
    strncpy((char *)data + OTA_STATUS_VERSION_OFFSET,
            OTA_FIRMWARE_VERSION, 16);

    write_le32(out + OTA_REPORT_CRC_OFFSET,
               ota_update_crc32(out, OTA_REPORT_CRC_OFFSET));
    __asm volatile("fence rw, rw" ::: "memory");
    ota_status_index = next;
}

ATTR_TCM_SECTION
void ota_update_get_status_report(uint8_t out[OTA_REPORT_PAYLOAD_SIZE])
{
    if (!out)
        return;
    uint8_t index = ota_status_index;
    __asm volatile("fence r, r" ::: "memory");
    memcpy(out, ota_status_reports[index], OTA_REPORT_PAYLOAD_SIZE);
}

static void set_maintenance(bool active)
{
    if (ota_maintenance == active)
        return;

    ota_maintenance = active;
    if (maintenance_hook)
        maintenance_hook(active);
}

bool ota_update_maintenance_active(void)
{
    return ota_maintenance;
}

void ota_update_set_maintenance_hook(ota_maintenance_hook_t hook)
{
    maintenance_hook = hook;
    if (hook && ota_maintenance)
        hook(true);
}

static void reset_transfer_fields(void)
{
    ota_ctx.session = 0;
    ota_ctx.accepted_offset = 0;
    ota_ctx.committed_offset = 0;
    ota_ctx.total_size = 0;
    ota_ctx.body_size = 0;
    ota_ctx.slice_len = 0;
    ota_ctx.auth_received = 0;
    ota_ctx.begin_flags = 0;
    ota_ctx.auth_required = false;
    ota_ctx.auth_verified = false;
    ota_ctx.header_checked = false;
    ota_ctx.reboot_pending = false;
    memset(ota_ctx.semver, 0, sizeof(ota_ctx.semver));
    memset(ota_ctx.signature, 0, sizeof(ota_ctx.signature));
    memset(ota_ctx.expected_body_sha256, 0,
           sizeof(ota_ctx.expected_body_sha256));
}

static void abort_sdk_handle(void)
{
    if (ota_ctx.sdk) {
        ota_abort(ota_ctx.sdk);
        ota_ctx.sdk = NULL;
    }
}

static void fail_transfer(uint8_t error)
{
    abort_sdk_handle();
    ota_ctx.state = OTA_STATE_ERROR;
    ota_ctx.error = error;
    ota_ctx.reboot_pending = false;
    set_maintenance(false);
    publish_status();
}

static bool validate_frame(const uint8_t *report, uint8_t *error)
{
    uint8_t data_len;

    if (read_le32(report + OTA_REPORT_CRC_OFFSET) !=
        ota_update_crc32(report, OTA_REPORT_CRC_OFFSET)) {
        *error = OTA_ERROR_BAD_CRC;
        return false;
    }
    if (report[0] != OTA_FRAME_MAGIC_0 ||
        report[1] != OTA_FRAME_MAGIC_1) {
        *error = OTA_ERROR_BAD_MAGIC;
        return false;
    }
    if (report[2] != OTA_PROTOCOL_VERSION) {
        *error = OTA_ERROR_BAD_VERSION;
        return false;
    }

    data_len = report[12];
    if (data_len > OTA_DATA_BYTES_MAX) {
        *error = OTA_ERROR_BAD_LENGTH;
        return false;
    }
    if (!bytes_are_zero(report + 13u + data_len,
                        OTA_REPORT_CRC_OFFSET - (13u + data_len))) {
        *error = OTA_ERROR_BAD_LENGTH;
        return false;
    }
    return true;
}

static bool validate_ota_header(void)
{
    const uint8_t *header = ota_ctx.slice;
    uint32_t body_size;
    uint8_t header_semver[3];

    if (ota_ctx.slice_len < OTA_BOUFFALO_HEADER_SIZE)
        return false;
    if (memcmp(header, "BL60X_OTA_Ver1.0", 16) != 0)
        return false;
    if (memcmp(header + 16, "RAW ", 4) != 0)
        return false;

    body_size = read_le32(header + 20);
    if (body_size != ota_ctx.body_size ||
        body_size > UINT32_MAX - OTA_BOUFFALO_HEADER_SIZE ||
        body_size + OTA_BOUFFALO_HEADER_SIZE != ota_ctx.total_size)
        return false;
    if (memcmp(header + 64, ota_ctx.expected_body_sha256, 32) != 0)
        return false;
    if (memcmp(header + 48, "EVENT_V", 7) != 0)
        return false;

    const uint8_t *cursor = header + 55;
    const uint8_t *version_end = header + 64;
    for (uint8_t component = 0; component < 3; component++) {
        uint32_t value = 0;
        uint8_t digits = 0;
        while (cursor < version_end && *cursor >= '0' && *cursor <= '9') {
            value = value * 10u + (uint32_t)(*cursor++ - '0');
            if (value > 254u)
                return false;
            digits++;
        }
        if (digits == 0)
            return false;
        header_semver[component] = (uint8_t)value;
        if (component < 2) {
            if (cursor >= version_end || *cursor++ != '.')
                return false;
        }
    }
    if (cursor < version_end && *cursor != 0)
        return false;
    while (cursor < version_end) {
        if (*cursor++ != 0)
            return false;
    }
    if (memcmp(header_semver, ota_ctx.semver, sizeof(header_semver)) != 0)
        return false;
    if (ota_ctx.sdk && body_size > ota_ctx.sdk->part_size)
        return false;

    ota_ctx.header_checked = true;
    return true;
}

static int commit_slice(void)
{
    if (ota_ctx.slice_len == 0)
        return 0;
    if (!ota_ctx.sdk)
        return -1;
    if (ota_update(ota_ctx.sdk, ota_ctx.slice, ota_ctx.slice_len) != 0)
        return -1;

    ota_ctx.committed_offset += ota_ctx.slice_len;
    ota_ctx.slice_len = 0;
    return 0;
}

static void process_data(const uint8_t *report)
{
    uint8_t error = OTA_ERROR_OK;
    uint32_t session;
    uint32_t offset;
    uint8_t data_len;

    if (!validate_frame(report, &error)) {
        ota_ctx.error = error;
        publish_status();
        return;
    }
    if (report[3] != OTA_MSG_DATA) {
        ota_ctx.error = OTA_ERROR_BAD_OPCODE;
        publish_status();
        return;
    }
    ota_ctx.last_command = OTA_MSG_DATA;
    if (ota_ctx.state != OTA_STATE_RECEIVING || !ota_ctx.sdk) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }

    session = read_le32(report + 4);
    offset = read_le32(report + 8);
    data_len = report[12];
    if (session != ota_ctx.session) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }
    if (offset != ota_ctx.accepted_offset) {
        ota_ctx.error = OTA_ERROR_BAD_OFFSET;
        publish_status();
        return;
    }
    if (data_len == 0 || ota_ctx.accepted_offset > ota_ctx.total_size ||
        data_len > ota_ctx.total_size - ota_ctx.accepted_offset) {
        ota_ctx.error = OTA_ERROR_BAD_LENGTH;
        publish_status();
        return;
    }

    ota_last_activity_tick = xTaskGetTickCount();

    const uint8_t *source = report + 13;
    uint32_t remaining = data_len;
    while (remaining > 0) {
        uint32_t room = OTA_SDK_SLICE_SIZE - ota_ctx.slice_len;
        uint32_t copy = remaining < room ? remaining : room;
        memcpy(ota_ctx.slice + ota_ctx.slice_len, source, copy);
        ota_ctx.slice_len += copy;
        ota_ctx.accepted_offset += copy;
        source += copy;
        remaining -= copy;

        if (!ota_ctx.header_checked &&
            ota_ctx.accepted_offset >= OTA_BOUFFALO_HEADER_SIZE &&
            !validate_ota_header()) {
            fail_transfer(OTA_ERROR_HEADER);
            return;
        }

        if (ota_ctx.slice_len == OTA_SDK_SLICE_SIZE) {
            if (commit_slice() != 0) {
                fail_transfer(ota_ctx.header_checked ? OTA_ERROR_FLASH
                                                     : OTA_ERROR_HEADER);
                return;
            }
        }
    }

    ota_ctx.error = OTA_ERROR_OK;
    publish_status();
}

static bool begin_matches_live_session(const uint8_t *report)
{
    const uint8_t *data = report + 13;
    bool live = ota_ctx.state == OTA_STATE_AUTHORIZING ||
                ota_ctx.state == OTA_STATE_PREPARING ||
                ota_ctx.state == OTA_STATE_RECEIVING;

    return live &&
           read_le32(report + 4) == ota_ctx.session &&
           read_le32(report + 8) == ota_ctx.total_size &&
           data[OTA_BEGIN_BOARD_OFFSET] == OTA_CURRENT_BOARD &&
           data[OTA_BEGIN_SPEED_OFFSET] == OTA_CURRENT_USB_SPEED &&
           memcmp(data + OTA_BEGIN_SEMVER_OFFSET, ota_ctx.semver, 3) == 0 &&
           data[OTA_BEGIN_FLAGS_OFFSET] == ota_ctx.begin_flags &&
           read_le32(data + OTA_BEGIN_BODY_LEN_OFFSET) == ota_ctx.body_size &&
           memcmp(data + OTA_BEGIN_BODY_SHA_OFFSET,
                  ota_ctx.expected_body_sha256, 32) == 0;
}

static void start_receiving_after_auth(void)
{
    ota_ctx.state = OTA_STATE_PREPARING;
    ota_ctx.error = OTA_ERROR_OK;
    set_maintenance(true);
    publish_status();

    /* CONFIG_FAST_OTA erases only the inactive FW slot.  It is deliberately
     * called after authentication because it may take several seconds. */
    ota_ctx.sdk = ota_start();
    if (!ota_ctx.sdk) {
        fail_transfer(OTA_ERROR_FLASH);
        return;
    }
    if (ota_ctx.body_size > ota_ctx.sdk->part_size) {
        fail_transfer(OTA_ERROR_TOO_LARGE);
        return;
    }

    ota_last_activity_tick = xTaskGetTickCount();
    ota_ctx.state = OTA_STATE_RECEIVING;
    ota_ctx.error = OTA_ERROR_OK;
    publish_status();
    LOG_INF("[OTA] session %lu authenticated, size=%lu inactive=0x%08lx\n",
            (unsigned long)ota_ctx.session,
            (unsigned long)ota_ctx.total_size,
            (unsigned long)ota_ctx.sdk->ota_addr);
}

static void process_begin(const uint8_t *report)
{
    const uint8_t *data = report + 13;
    uint32_t session = read_le32(report + 4);
    uint32_t total_size = read_le32(report + 8);
    uint32_t body_size = read_le32(data + OTA_BEGIN_BODY_LEN_OFFSET);
    uint8_t flags = data[OTA_BEGIN_FLAGS_OFFSET];
    bool signed_image = (flags & OTA_BEGIN_FLAG_SIGNED) != 0;

    if (report[12] != OTA_BEGIN_DATA_SIZE) {
        ota_ctx.error = OTA_ERROR_BAD_LENGTH;
        publish_status();
        return;
    }
    if (begin_matches_live_session(report)) {
        ota_last_activity_tick = xTaskGetTickCount();
        ota_ctx.error = OTA_ERROR_OK;
        publish_status();
        return;
    }

    if (ota_ctx.state == OTA_STATE_AUTHORIZING ||
        ota_ctx.state == OTA_STATE_PREPARING ||
        ota_ctx.state == OTA_STATE_RECEIVING ||
        ota_ctx.state == OTA_STATE_VERIFYING ||
        ota_ctx.state == OTA_STATE_READY_REBOOT || trial_boot) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }
    if (session == 0) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }
    if (data[OTA_BEGIN_BOARD_OFFSET] != OTA_CURRENT_BOARD ||
        data[OTA_BEGIN_SPEED_OFFSET] != OTA_CURRENT_USB_SPEED ||
        (flags & (uint8_t)~OTA_BEGIN_FLAG_SIGNED) != 0 ||
        data[OTA_BEGIN_SEMVER_OFFSET] == 0xFFu ||
        data[OTA_BEGIN_SEMVER_OFFSET + 1] == 0xFFu ||
        data[OTA_BEGIN_SEMVER_OFFSET + 2] == 0xFFu ||
        !semver_fits_ota_header(data + OTA_BEGIN_SEMVER_OFFSET)) {
        ota_ctx.error = OTA_ERROR_BAD_TARGET;
        publish_status();
        return;
    }
    if (data[OTA_BEGIN_SEMVER_OFFSET] < APP_VER_X ||
        (data[OTA_BEGIN_SEMVER_OFFSET] == APP_VER_X &&
         data[OTA_BEGIN_SEMVER_OFFSET + 1] < APP_VER_Y) ||
        (data[OTA_BEGIN_SEMVER_OFFSET] == APP_VER_X &&
         data[OTA_BEGIN_SEMVER_OFFSET + 1] == APP_VER_Y &&
         data[OTA_BEGIN_SEMVER_OFFSET + 2] <= APP_VER_Z)) {
        ota_ctx.error = OTA_ERROR_BAD_TARGET;
        publish_status();
        return;
    }
    if (body_size == 0 || body_size > UINT32_MAX - OTA_BOUFFALO_HEADER_SIZE ||
        body_size + OTA_BOUFFALO_HEADER_SIZE != total_size) {
        ota_ctx.error = OTA_ERROR_BAD_LENGTH;
        publish_status();
        return;
    }
    if (ota_ctx.max_image_size == 0 || total_size > ota_ctx.max_image_size) {
        ota_ctx.error = OTA_ERROR_TOO_LARGE;
        publish_status();
        return;
    }
    if (bytes_are_zero(data + OTA_BEGIN_BODY_SHA_OFFSET, 32)) {
        ota_ctx.error = OTA_ERROR_HASH;
        publish_status();
        return;
    }
    if ((stable_signature_required() || signed_image) && !release_key_ready) {
        ota_ctx.error = OTA_ERROR_KEY_MISSING;
        publish_status();
        return;
    }
    if (stable_signature_required() && !signed_image) {
        ota_ctx.error = OTA_ERROR_AUTH_REQUIRED;
        publish_status();
        return;
    }

    abort_sdk_handle();
    reset_transfer_fields();
    ota_ctx.session = session;
    ota_ctx.total_size = total_size;
    ota_ctx.body_size = body_size;
    ota_ctx.begin_flags = flags;
    memcpy(ota_ctx.semver, data + OTA_BEGIN_SEMVER_OFFSET, 3);
    memcpy(ota_ctx.expected_body_sha256,
           data + OTA_BEGIN_BODY_SHA_OFFSET, 32);
    ota_ctx.auth_required = stable_signature_required() || signed_image;
    ota_ctx.auth_verified = !ota_ctx.auth_required;
    ota_last_activity_tick = xTaskGetTickCount();
    ota_ctx.error = OTA_ERROR_OK;

    if (ota_ctx.auth_required) {
        ota_ctx.state = OTA_STATE_AUTHORIZING;
        publish_status();
        LOG_INF("[OTA] session %lu waiting for release signature\n",
                (unsigned long)ota_ctx.session);
    } else {
        LOG_WRN("[OTA] UNSAFE unsigned development OTA accepted\n");
        start_receiving_after_auth();
    }
}

static void process_auth(const uint8_t *report)
{
    uint32_t signature_offset = read_le32(report + 8);
    uint8_t data_len = report[12];
    const uint8_t *data = report + 13;
    uint8_t expected_len;

    if (read_le32(report + 4) != ota_ctx.session || ota_ctx.session == 0) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }
    if (!ota_ctx.auth_required) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }
    if (ota_ctx.state != OTA_STATE_AUTHORIZING &&
        !(ota_ctx.state == OTA_STATE_RECEIVING && ota_ctx.auth_verified)) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }
    if (!release_key_ready) {
        ota_ctx.error = OTA_ERROR_KEY_MISSING;
        publish_status();
        return;
    }

    if (signature_offset == 0)
        expected_len = 46;
    else if (signature_offset == 46)
        expected_len = 18;
    else {
        ota_ctx.error = OTA_ERROR_BAD_OFFSET;
        publish_status();
        return;
    }
    if (data_len != expected_len) {
        ota_ctx.error = OTA_ERROR_BAD_LENGTH;
        publish_status();
        return;
    }

    ota_last_activity_tick = xTaskGetTickCount();

    if (signature_offset == 0) {
        if (ota_ctx.auth_received >= 46) {
            if (memcmp(ota_ctx.signature, data, 46) != 0) {
                ota_ctx.error = OTA_ERROR_BAD_OFFSET;
                publish_status();
                return;
            }
        } else {
            memcpy(ota_ctx.signature, data, 46);
            ota_ctx.auth_received = 46;
        }
    } else {
        if (ota_ctx.auth_received < 46) {
            ota_ctx.error = OTA_ERROR_BAD_OFFSET;
            publish_status();
            return;
        }
        if (ota_ctx.auth_received == OTA_AUTH_SIGNATURE_SIZE) {
            if (memcmp(ota_ctx.signature + 46, data, 18) != 0) {
                ota_ctx.error = OTA_ERROR_BAD_OFFSET;
                publish_status();
                return;
            }
        } else {
            memcpy(ota_ctx.signature + 46, data, 18);
            ota_ctx.auth_received = OTA_AUTH_SIGNATURE_SIZE;
        }
    }

    ota_ctx.error = OTA_ERROR_OK;
    if (ota_ctx.auth_verified ||
        ota_ctx.auth_received != OTA_AUTH_SIGNATURE_SIZE) {
        publish_status();
        return;
    }

    if (!verify_signature()) {
        fail_transfer(OTA_ERROR_AUTH_FAILED);
        return;
    }

    ota_ctx.auth_verified = true;
    start_receiving_after_auth();
}

static void process_commit(uint32_t session)
{
    ota_ctx.last_command = OTA_CTRL_COMMIT;
    if (ota_ctx.state != OTA_STATE_RECEIVING || !ota_ctx.sdk) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }
    if (session != ota_ctx.session) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }
    if (!ota_ctx.auth_verified) {
        ota_ctx.error = OTA_ERROR_AUTH_REQUIRED;
        publish_status();
        return;
    }
    if (ota_ctx.accepted_offset != ota_ctx.total_size ||
        !ota_ctx.header_checked) {
        ota_ctx.error = OTA_ERROR_BAD_LENGTH;
        publish_status();
        return;
    }

    ota_ctx.state = OTA_STATE_VERIFYING;
    ota_ctx.error = OTA_ERROR_OK;
    publish_status();

    if (commit_slice() != 0) {
        fail_transfer(OTA_ERROR_FLASH);
        return;
    }

    /* Boot2 decrements a retry marker above 0xF0 on every attempted boot.
     * 0xF2 allows two failed trial boots before Boot2 rolls back. */
    ota_ctx.sdk->pt_fw_entry.age =
        (ota_ctx.sdk->pt_fw_entry.age & 0x00FFFFFFu) |
        ((uint32_t)OTA_TRIAL_RETRY_BYTE << 24);

    uint8_t next_slot = (ota_ctx.sdk->pt_fw_entry.active_index & 1u) ^ 1u;
    if (ota_finish(ota_ctx.sdk, 1, 0) != 0) {
        /* ota_finish retains ownership when hash/PT publication fails. */
        abort_sdk_handle();
        fail_transfer(OTA_ERROR_HASH);
        return;
    }
    ota_ctx.sdk = NULL;
    ota_ctx.committed_offset = ota_ctx.total_size;
    ota_ctx.active_slot = next_slot;
    ota_ctx.trial_retry_byte = OTA_TRIAL_RETRY_BYTE;
    ota_ctx.state = OTA_STATE_READY_REBOOT;
    ota_ctx.error = OTA_ERROR_OK;
    ota_ctx.reboot_pending = true;
    reboot_at_tick = xTaskGetTickCount() +
                     pdMS_TO_TICKS(OTA_REBOOT_DELAY_100MS * 100u);
    publish_status();
    LOG_INF("[OTA] image verified; slot %u selected, reboot in %u00 ms\n",
            next_slot, OTA_REBOOT_DELAY_100MS);
}

static void process_abort(uint32_t session)
{
    ota_ctx.last_command = OTA_CTRL_ABORT;
    if (ota_ctx.reboot_pending || ota_ctx.state == OTA_STATE_READY_REBOOT ||
        ota_ctx.state == OTA_STATE_VERIFYING) {
        ota_ctx.error = OTA_ERROR_BAD_STATE;
        publish_status();
        return;
    }
    if (session != ota_ctx.session) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }

    abort_sdk_handle();
    reset_transfer_fields();
    ota_ctx.state = OTA_STATE_ABORTED;
    ota_ctx.error = OTA_ERROR_OK;
    ota_ctx.last_command = OTA_CTRL_ABORT;
    set_maintenance(false);
    publish_status();
    LOG_INF("[OTA] WebHID transfer aborted\n");
}

static void process_status(uint32_t session)
{
    if (session != 0 && session != ota_ctx.session) {
        ota_ctx.error = OTA_ERROR_BAD_SESSION;
        publish_status();
        return;
    }
    /* STATUS is observational.  Keep the last error sticky so SET_REPORT
     * followed by GET_REPORT cannot erase a BEGIN/AUTH/DATA failure before
     * the browser has had a chance to read it.  A successful mutating
     * command clears the error in its own handler. */
    publish_status();
}

static void process_control(const uint8_t *report)
{
    uint8_t error = OTA_ERROR_OK;
    uint8_t command;
    uint32_t session;
    uint32_t arg;
    uint8_t data_len;

    if (!validate_frame(report, &error)) {
        ota_ctx.error = error;
        publish_status();
        return;
    }

    command = report[3];
    session = read_le32(report + 4);
    arg = read_le32(report + 8);
    data_len = report[12];
    ota_ctx.last_command = command;

    switch (command) {
    case OTA_CTRL_BEGIN:
        process_begin(report);
        break;
    case OTA_CTRL_AUTH:
        process_auth(report);
        break;
    case OTA_CTRL_COMMIT:
        if (arg != 0 || data_len != 0) {
            ota_ctx.error = OTA_ERROR_BAD_LENGTH;
            publish_status();
        } else {
            process_commit(session);
        }
        break;
    case OTA_CTRL_ABORT:
        if (arg != 0 || data_len != 0) {
            ota_ctx.error = OTA_ERROR_BAD_LENGTH;
            publish_status();
        } else {
            process_abort(session);
        }
        break;
    case OTA_CTRL_STATUS:
        if (arg != 0 || data_len != 0) {
            ota_ctx.error = OTA_ERROR_BAD_LENGTH;
            publish_status();
        } else {
            process_status(session);
        }
        break;
    default:
        ota_ctx.error = OTA_ERROR_BAD_OPCODE;
        publish_status();
        break;
    }
}

ATTR_TCM_SECTION
static bool enqueue_from_isr(uint8_t kind, const uint8_t *payload,
                             uint32_t len)
{
    BaseType_t woken = pdFALSE;
    ota_event_t event;

    if (!ota_event_queue || !payload || len != OTA_REPORT_PAYLOAD_SIZE)
        return false;

    event.kind = kind;
    memcpy(event.payload, payload, OTA_REPORT_PAYLOAD_SIZE);
    if (xQueueSendFromISR(ota_event_queue, &event, &woken) != pdTRUE) {
        ota_queue_overflow = true;
        return false;
    }
    portYIELD_FROM_ISR(woken);
    return true;
}

bool ota_update_enqueue_data_from_isr(const uint8_t *payload, uint32_t len)
{
    return enqueue_from_isr(OTA_EVENT_DATA, payload, len);
}

bool ota_update_enqueue_control_from_isr(const uint8_t *payload, uint32_t len)
{
    return enqueue_from_isr(OTA_EVENT_CONTROL, payload, len);
}

static void start_trial_watchdog_if_needed(void)
{
    if (!trial_boot)
        return;

    trial_wdg = bflb_device_get_by_name("watchdog0");
    if (!trial_wdg) {
        LOG_WRN("[OTA] trial image: watchdog0 unavailable\n");
        return;
    }

    struct bflb_wdg_config_s config = {
        .clock_source = WDG_CLKSRC_32K,
        .clock_div = 31,
        .comp_val = OTA_TRIAL_WDG_SECONDS * 1000u,
        .mode = WDG_MODE_RESET,
    };
    bflb_wdg_init(trial_wdg, &config);
    bflb_wdg_start(trial_wdg);
    trial_watchdog_started = true;
    LOG_INF("[OTA] trial boot retry=0x%02x, %us watchdog armed\n",
            ota_ctx.trial_retry_byte, OTA_TRIAL_WDG_SECONDS);
}

void ota_update_notify_runtime_healthy(void)
{
    runtime_healthy_tick = xTaskGetTickCount();
    __asm volatile("fence rw, rw" ::: "memory");
    runtime_healthy = true;
}

static void service_trial_boot(void)
{
    if (!trial_boot)
        return;

    TickType_t now = xTaskGetTickCount();
    TickType_t elapsed = now - trial_task_start_tick;
    bool within_feed_window =
        elapsed < pdMS_TO_TICKS(OTA_TRIAL_FEED_LIMIT_MS);

    if (trial_watchdog_started && within_feed_window &&
        (now - trial_last_feed_tick) >= pdMS_TO_TICKS(500)) {
        bflb_wdg_reset_countervalue(trial_wdg);
        trial_last_feed_tick = now;
    }

    if (!runtime_healthy ||
        (now - runtime_healthy_tick) <
            pdMS_TO_TICKS(OTA_TRIAL_HEALTH_HOLD_MS)) {
        return;
    }

    if (pt_table_set_fw_boot_success(PT_ENTRY_FW_CPU0) != PT_ERROR_SUCCESS) {
        LOG_ERR("[OTA] failed to confirm trial image; watchdog remains armed\n");
        return;
    }

    if (trial_watchdog_started) {
        bflb_wdg_stop(trial_wdg);
        trial_watchdog_started = false;
    }
    trial_boot = false;
    ota_ctx.trial_retry_byte = 0;
    publish_status();
    LOG_INF("[OTA] runtime healthy; trial image confirmed\n");
}

int ota_update_init(void)
{
    memset(&ota_ctx, 0, sizeof(ota_ctx));
    memset(ota_status_reports, 0, sizeof(ota_status_reports));
    ota_status_index = 0;
    ota_queue_overflow = false;
    ota_maintenance = false;
    runtime_healthy = false;
    maintenance_hook = NULL;
    ota_ctx.state = OTA_STATE_IDLE;
    ota_ctx.error = OTA_ERROR_OK;

    release_key_ready = release_public_key_is_valid();
    refresh_partition_info();
    /* Boot2 decrements F2 -> F1 before the first trial boot and F1 -> F0
     * before the second.  F0 is therefore still a live trial attempt and
     * must be confirmed by the application before the next reboot. */
    trial_boot = ota_ctx.trial_retry_byte >= 0xF0u;
    start_trial_watchdog_if_needed();

    ota_event_queue = xQueueCreate(OTA_EVENT_QUEUE_DEPTH,
                                   sizeof(ota_event_t));
    if (!ota_event_queue) {
        ota_ctx.state = OTA_STATE_ERROR;
        ota_ctx.error = OTA_ERROR_INTERNAL;
        publish_status();
        return -1;
    }

    publish_status();
    LOG_INF("[OTA] WebHID A/B ready: board=%u usb=%u max=%lu key=%s sig=%s\n",
            OTA_CURRENT_BOARD, OTA_CURRENT_USB_SPEED,
            (unsigned long)ota_ctx.max_image_size,
            release_key_ready ? "configured" : "missing",
            stable_signature_required() ? "required" : "DEV-OPTIONAL");
    return 0;
}

void ota_update_task(void *arg)
{
    (void)arg;
    ota_event_t event;

    trial_task_start_tick = xTaskGetTickCount();
    trial_last_feed_tick = trial_task_start_tick;
    ota_last_activity_tick = trial_task_start_tick;

    for (;;) {
        service_trial_boot();

        TickType_t now = xTaskGetTickCount();
        if (ota_ctx.session != 0 &&
            (ota_ctx.state == OTA_STATE_AUTHORIZING ||
             ota_ctx.state == OTA_STATE_RECEIVING) &&
            (now - ota_last_activity_tick) >=
                pdMS_TO_TICKS(OTA_SESSION_IDLE_TIMEOUT_MS)) {
            LOG_WRN("[OTA] session timed out; aborting inactive transfer\n");
            fail_transfer(OTA_ERROR_TIMEOUT);
        }

        if (ota_ctx.reboot_pending &&
            (int32_t)(xTaskGetTickCount() - reboot_at_tick) >= 0) {
            LOG_INF("[OTA] rebooting into trial slot %u\n",
                    ota_ctx.active_slot);
            bl_sys_reset_por();
        }

        if (ota_queue_overflow) {
            taskENTER_CRITICAL();
            ota_queue_overflow = false;
            taskEXIT_CRITICAL();
            ota_ctx.error = OTA_ERROR_QUEUE_FULL;
            publish_status();
        }

        if (xQueueReceive(ota_event_queue, &event,
                          pdMS_TO_TICKS(20)) != pdTRUE) {
            continue;
        }

        if (event.kind == OTA_EVENT_DATA)
            process_data(event.payload);
        else if (event.kind == OTA_EVENT_CONTROL)
            process_control(event.payload);
    }
}
