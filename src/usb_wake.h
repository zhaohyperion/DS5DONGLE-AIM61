#ifndef USB_WAKE_H
#define USB_WAKE_H

#include <stdint.h>
#include <stdbool.h>

void usb_wake_init(void);
void usb_wake_on_suspend(void);
void usb_wake_on_resume(void);
void usb_wake_on_configured(void);
void usb_wake_on_bt_connect(void);
void usb_wake_on_bt_disconnect(void);
bool usb_wake_host_suspended(void);

/**
 * Feed BT input to the wake FSM for button-change detection.
 * @param payload  USB-equivalent payload (63 bytes, starting from stick axes)
 * @param len      payload length (must be >= 10)
 */
void usb_wake_on_bt_input(const uint8_t *payload, uint16_t len);

/** Advance the wake FSM. Call periodically from the USB task loop. */
void usb_wake_task(void);

/**
 * Consume a pending request to re-enable BR/EDR scans after host resume.
 * Returns false while a newer suspend generation is active.
 */
bool usb_wake_take_radio_wake_request(void);

#endif /* USB_WAKE_H */
