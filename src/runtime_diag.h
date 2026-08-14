#ifndef RUNTIME_DIAG_H
#define RUNTIME_DIAG_H

#include <stdbool.h>
#include <stdint.h>

/* Runtime diagnostics use one vendor Feature report that is outside
 * the DualSense and existing dongle configuration/OTA report ranges.  The
 * HID report ID is handled separately by the HID layer and is not part of the
 * 63-byte payload described below. */
#define RUNTIME_DIAG_REPORT_ID          0xFDu
#define RUNTIME_DIAG_REPORT_SIZE        63u
#define RUNTIME_DIAG_CRC_OFFSET         59u
#define RUNTIME_DIAG_PROTOCOL_VERSION   2u
#define RUNTIME_DIAG_HEADER_SIZE        16u
#define RUNTIME_DIAG_DATA_OFFSET        16u
#define RUNTIME_DIAG_DATA_MAX           43u
#define RUNTIME_DIAG_PAGE_COUNT         7u
#define RUNTIME_DIAG_SELECT_PAGE        0x01u
#define RUNTIME_DIAG_SET_SESSION        0x02u
#define RUNTIME_DIAG_SELECT_SIZE        3u
#define RUNTIME_DIAG_PUBLISH_INTERVAL_MS 1000u

#define RUNTIME_DIAG_MAGIC_0 'D'
#define RUNTIME_DIAG_MAGIC_1 'G'

enum runtime_diag_page {
    RUNTIME_DIAG_PAGE_IDENTITY = 0,
    RUNTIME_DIAG_PAGE_USB_BT = 1,
    RUNTIME_DIAG_PAGE_PRESSURE = 2,
    RUNTIME_DIAG_PAGE_AUDIO_TIMING = 3,
    RUNTIME_DIAG_PAGE_AUDIO_PIPELINE = 4,
    RUNTIME_DIAG_PAGE_OTA_MEMORY = 5,
    RUNTIME_DIAG_PAGE_BRIDGE_LATENCY = 6,
};

#if DS5_DIAGNOSTIC_BUILD
/* Prepare valid page-zero-selected reports before USB enumeration starts. */
void runtime_diag_init(void);

/* Called by the dedicated low-priority diagnostic task.  A complete six-page
 * snapshot is generated at most once per interval, then atomically published. */
void runtime_diag_task_update(uint64_t monotonic_us);

/* EP0-safe operations: bounded validation plus one byte store for SET, and a
 * fixed 63-byte copy from the published bank for GET. */
bool runtime_diag_select_page_from_isr(const uint8_t *payload, uint32_t len);
void runtime_diag_get_selected_report(
    uint8_t out[RUNTIME_DIAG_REPORT_SIZE]);
#endif

#endif /* RUNTIME_DIAG_H */
