#ifndef DS5_MACRO_ENGINE_H
#define DS5_MACRO_ENGINE_H

#include <stdbool.h>
#include <stdint.h>

#define MACRO_REPORT_ID 0xFEu
#define MACRO_REPORT_SIZE 63u
#define MACRO_FORMAT_VERSION 3u
#define MACRO_MAX_BYTES 8192u

/* Call after EasyFlash/LittleFS has been initialized. */
int macro_engine_init(void);

/* Feature transport. SET is copied in the USB ISR and handled later by the
 * USB task. GET only copies an already-published fixed-size status snapshot. */
bool macro_engine_enqueue_from_isr(const uint8_t *payload, uint32_t len);
void macro_engine_process_deferred(void);
void macro_engine_get_report(uint8_t out[MACRO_REPORT_SIZE]);

/* Input pipeline: raw physical state must be observed before remapping; the
 * compiled macro is applied after global remapping and never remapped again. */
void macro_engine_observe_raw(const uint8_t *payload, uint64_t now_us);
void macro_engine_record_mapped(const uint8_t *payload, uint64_t now_us);
void macro_engine_apply(uint8_t *payload, uint64_t now_us);
void macro_engine_on_disconnect(void);
void macro_engine_stop_all(void);

#endif
