#ifndef OTA_UPDATE_H
#define OTA_UPDATE_H

#include <stdbool.h>
#include <stdint.h>

/* WebHID transport ---------------------------------------------------------
 *
 * Both reports contain exactly 63 payload bytes.  The HID report ID is not
 * part of these layouts (WebHID supplies it separately).
 *
 * DATA (Output report 0xFA):
 *   0..1   "OT"
 *   2      protocol version (1)
 *   3      OTA_MSG_DATA (0x10)
 *   4..7   session, little endian
 *   8..11  byte offset in the complete .bin.ota file, little endian
 *   12     data length (0..46)
 *   13..58 data, zero padded
 *   59..62 CRC32/IEEE of bytes 0..58, little endian
 *
 * CONTROL (Feature report 0xFC, SET_REPORT) uses the same common layout as
 * DATA: bytes 8..11 are command arg, byte 12 is data length (0..46), and
 * bytes 13..58 are command data.  BEGIN metadata is documented by the
 * OTA_BEGIN_* offsets below.  AUTH carries a raw P-256 r||s signature in two
 * frames and uses arg as signature offset.  STATUS with session=0 is CAPS.
 *
 * GET_REPORT returns ACK (0x80) or ERROR (0xFF): arg is accepted_offset,
 * len=44, and data is enum ota_status_data_offset.  All request/response
 * frames store CRC32/IEEE(bytes 0..58) little-endian at bytes 59..62.
 */

#define OTA_DATA_REPORT_ID       0xFAu
#define OTA_CONTROL_REPORT_ID    0xFCu
#define OTA_REPORT_PAYLOAD_SIZE  63u
#define OTA_REPORT_CRC_OFFSET    59u
#define OTA_PROTOCOL_VERSION     1u
#define OTA_DATA_BYTES_MAX       46u
#define OTA_SDK_SLICE_SIZE       4096u
#define OTA_BOUFFALO_HEADER_SIZE 512u
#define OTA_REBOOT_DELAY_100MS   15u
#define OTA_WINDOW_FRAMES        16u
#define OTA_AUTH_SIGNATURE_SIZE  64u
#define OTA_SIGN_CANONICAL_SIZE  57u
#define OTA_BEGIN_DATA_SIZE      42u
#define OTA_STATUS_DATA_SIZE     44u

#define OTA_FRAME_MAGIC_0 'O'
#define OTA_FRAME_MAGIC_1 'T'

enum ota_message_type {
    OTA_MSG_DATA = 0x10,
};

enum ota_control_command {
    OTA_CTRL_BEGIN  = 0x01,
    OTA_CTRL_AUTH   = 0x02,
    OTA_CTRL_COMMIT = 0x03,
    OTA_CTRL_ABORT  = 0x04,
    OTA_CTRL_STATUS = 0x05,
    OTA_CTRL_ACK    = 0x80,
    OTA_CTRL_ERROR  = 0xFF,
};

enum ota_update_state {
    OTA_STATE_IDLE         = 0,
    OTA_STATE_AUTHORIZING  = 1,
    OTA_STATE_PREPARING    = 2,
    OTA_STATE_RECEIVING    = 3,
    OTA_STATE_VERIFYING    = 4,
    OTA_STATE_READY_REBOOT = 5,
    OTA_STATE_ERROR        = 6,
    OTA_STATE_ABORTED      = 7,
};

enum ota_update_error {
    OTA_ERROR_OK          = 0,
    OTA_ERROR_BAD_MAGIC   = 1,
    OTA_ERROR_BAD_VERSION = 2,
    OTA_ERROR_BAD_OPCODE  = 3,
    OTA_ERROR_BAD_STATE   = 4,
    OTA_ERROR_BAD_SESSION = 5,
    OTA_ERROR_BAD_OFFSET  = 6,
    OTA_ERROR_BAD_LENGTH  = 7,
    OTA_ERROR_BAD_CRC     = 8,
    OTA_ERROR_BAD_TARGET  = 9,
    OTA_ERROR_TOO_LARGE   = 10,
    OTA_ERROR_QUEUE_FULL  = 11,
    OTA_ERROR_FLASH       = 12,
    OTA_ERROR_HASH        = 13,
    OTA_ERROR_HEADER      = 14,
    OTA_ERROR_INTERNAL    = 15,
    OTA_ERROR_AUTH_REQUIRED = 16,
    OTA_ERROR_AUTH_FAILED   = 17,
    OTA_ERROR_KEY_MISSING   = 18,
    OTA_ERROR_TIMEOUT       = 19,
};

enum ota_board_id {
    OTA_BOARD_AIM61     = 1,
    OTA_BOARD_LCTECH616 = 2,
    OTA_BOARD_M0S_DOCK  = 3,
};

enum ota_usb_speed {
    OTA_USB_FULL_SPEED = 0,
    OTA_USB_HIGH_SPEED = 1,
};

enum ota_image_format {
    OTA_FORMAT_RAW_OTA = 1,
};

enum ota_capability_bits {
    OTA_CAP_AB_SLOTS        = 1u << 0,
    OTA_CAP_SHA256          = 1u << 1,
    OTA_CAP_LIVE_RESUME     = 1u << 2,
    OTA_CAP_TRIAL_BOOT      = 1u << 3,
    OTA_CAP_MAINTENANCE     = 1u << 4,
    OTA_CAP_RAW_OTA         = 1u << 5,
    OTA_CAP_FRAME_CRC32     = 1u << 6,
    OTA_CAP_DELAYED_REBOOT  = 1u << 7,
    OTA_CAP_SIGNATURE_REQUIRED = 1u << 8,
    OTA_CAP_KEY_CONFIGURED     = 1u << 9,
};

enum ota_begin_data_offset {
    OTA_BEGIN_BOARD_OFFSET       = 0,
    OTA_BEGIN_SPEED_OFFSET       = 1,
    OTA_BEGIN_SEMVER_OFFSET      = 2,  /* 3 bytes */
    OTA_BEGIN_FLAGS_OFFSET       = 5,
    OTA_BEGIN_BODY_LEN_OFFSET    = 6,  /* uint32 LE */
    OTA_BEGIN_BODY_SHA_OFFSET    = 10, /* 32 bytes */
};

enum ota_begin_flags {
    OTA_BEGIN_FLAG_SIGNED = 1u << 0,
};

enum ota_status_data_offset {
    OTA_STATUS_STATE_OFFSET       = 0,
    OTA_STATUS_ERROR_OFFSET       = 1,
    OTA_STATUS_LAST_REQUEST       = 2,
    OTA_STATUS_FLAGS_OFFSET       = 3,
    OTA_STATUS_COMMITTED_OFFSET   = 4,
    OTA_STATUS_TOTAL_OFFSET       = 8,
    OTA_STATUS_MAX_FILE_OFFSET    = 12,
    OTA_STATUS_CAPABILITIES       = 16,
    OTA_STATUS_BOARD_OFFSET       = 20,
    OTA_STATUS_SPEED_OFFSET       = 21,
    OTA_STATUS_FORMAT_OFFSET      = 22,
    OTA_STATUS_MAX_DATA_OFFSET    = 23,
    OTA_STATUS_WINDOW_OFFSET      = 24,
    OTA_STATUS_ACTIVE_SLOT        = 25,
    OTA_STATUS_TRIAL_RETRY        = 26,
    OTA_STATUS_REBOOT_DELAY       = 27,
    OTA_STATUS_VERSION_OFFSET     = 28, /* 16 bytes */
};

enum ota_status_flags {
    OTA_STATUS_MAINTENANCE  = 1u << 0,
    OTA_STATUS_REBOOT_WAIT  = 1u << 1,
    OTA_STATUS_TRIAL_BOOT   = 1u << 2,
};

#define OTA_STRINGIFY_INNER(value) #value
#define OTA_STRINGIFY(value) OTA_STRINGIFY_INNER(value)
#define OTA_FIRMWARE_VERSION OTA_STRINGIFY(APP_VER_X) "." \
                             OTA_STRINGIFY(APP_VER_Y) "." \
                             OTA_STRINGIFY(APP_VER_Z)

#if defined(BOARD_LCTECH_616)
#define OTA_CURRENT_BOARD OTA_BOARD_LCTECH616
#elif defined(BOARD_M0S_DOCK)
#define OTA_CURRENT_BOARD OTA_BOARD_M0S_DOCK
#else
#define OTA_CURRENT_BOARD OTA_BOARD_AIM61
#endif

#ifdef FORCE_FS_MODE
#define OTA_CURRENT_USB_SPEED OTA_USB_FULL_SPEED
#else
#define OTA_CURRENT_USB_SPEED OTA_USB_HIGH_SPEED
#endif

typedef void (*ota_maintenance_hook_t)(bool active);

/* Called after bflb_mtd_init(), before the scheduler and before USB starts. */
int ota_update_init(void);

/* Dedicated OTA task.  All CRC/header/flash/hash/PT operations happen here. */
void ota_update_task(void *arg);

/* USB callback entry points: fixed-size copy to the dedicated OTA queue only. */
bool ota_update_enqueue_data_from_isr(const uint8_t *payload, uint32_t len);
bool ota_update_enqueue_control_from_isr(const uint8_t *payload, uint32_t len);

/* ISR-safe snapshot copy for GET_REPORT(0xFC). */
void ota_update_get_status_report(uint8_t out[OTA_REPORT_PAYLOAD_SIZE]);

bool ota_update_maintenance_active(void);
void ota_update_set_maintenance_hook(ota_maintenance_hook_t hook);

/* usb_task calls this only after BT clock setup and USB registration succeed.
 * On a trial image, the OTA task waits for this health point before clearing
 * Boot2's retry marker and stopping the trial watchdog. */
void ota_update_notify_runtime_healthy(void);

/* Exposed for protocol tests and the matching web implementation. */
uint32_t ota_update_crc32(const uint8_t *data, uint32_t len);

#endif /* OTA_UPDATE_H */
