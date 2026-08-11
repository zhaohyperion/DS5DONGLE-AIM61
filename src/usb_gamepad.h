#ifndef USB_GAMEPAD_H
#define USB_GAMEPAD_H

#include <stdint.h>
#include <stdbool.h>

#define USB_GAMEPAD_EP_IN       0x84
#define USB_GAMEPAD_EP_OUT      0x03
#define USB_KBD_EP_IN           0x83  /* EP3 IN — shares FIFO F2 (BID) with EP3 OUT (gamepad) */
#define USB_GAMEPAD_EP_MPS      64
#define USB_KBD_EP_MPS          8
#define USB_GAMEPAD_INTERVAL_MS 1
#define USB_KBD_INTERVAL_MS     10

#define USB_GAMEPAD_VID         0x054C
#define USB_GAMEPAD_PID         0x0CE6

#define USB_INTF_GAMEPAD        3   /* shifted: 0=AC, 1=AS_SPK, 2=AS_MIC, 3=Gamepad */
#define USB_INTF_KBD            4

#define DS5_USB_REPORT_ID_INPUT   0x01
#define DS5_USB_REPORT_ID_OUTPUT  0x02
#define DS5_USB_INPUT_PAYLOAD_LEN  63
#define DS5_USB_OUTPUT_PAYLOAD_LEN 47
#define FEATURE_DATA_MAX           256

typedef void (*usb_gamepad_output_cb_t)(const uint8_t *data, uint16_t len);

/* Cumulative, best-effort counters for the HID IN realtime path.  They are
 * deliberately 32-bit so snapshots are single-copy operations on BL618. */
struct usb_gamepad_runtime_stats {
    uint32_t input_updates;
    uint32_t input_coalesced;
    uint32_t transfers_started;
    uint32_t transfers_completed;
    uint32_t start_errors;
};

/* Low-rate task-context view of USB lifecycle state.  No register reads or
 * formatting occur while the USB callback state is sampled. */
struct usb_gamepad_link_status {
    bool configured;
    bool suspended;
    bool maintenance;
    bool dse_mode;
    bool input_busy;
    bool keyboard_registered;
};

int usb_gamepad_init(usb_gamepad_output_cb_t output_cb);

/* Two-step task-context publish API.  Staging writes only the inactive slot;
 * commit atomically publishes it and owns the endpoint kick.  This lets the
 * caller perform a connection-epoch check in the same critical section as
 * commit without masking the 63-byte staging copy. */
int usb_gamepad_stage_raw_input(const uint8_t *payload, uint8_t *slot);
int usb_gamepad_commit_raw_input(uint8_t slot);

/* Convenience wrapper for producers that do not need an external epoch check.
 * The firmware keeps usb_task as the sole producer. */
int usb_gamepad_send_raw_input(const uint8_t *payload);
bool usb_gamepad_is_ready(void);
int  usb_gamepad_send_kbd_report(const uint8_t *report, uint8_t len);
bool usb_gamepad_kbd_ready(void);
bool usb_gamepad_kbd_ready_at(uint64_t now_us);

/* Task-context snapshot used by the low-rate diagnostics path. */
void usb_gamepad_get_runtime_stats(struct usb_gamepad_runtime_stats *out);
void usb_gamepad_get_link_status(struct usb_gamepad_link_status *out);

void usb_gamepad_set_suspend_hooks(void (*on_suspend)(void),
                                   void (*on_resume)(void),
                                   void (*on_configured)(void));

void usb_gamepad_set_polling_rate(uint8_t mode);

void usb_gamepad_set_dse_mode(bool dse);

/* Stop DS5/keyboard traffic while preserving WebHID OTA reports. */
void usb_gamepad_set_maintenance_mode(bool active);

void usb_gamepad_process_deferred(void);

void usb_soft_disconnect(void);
void usb_soft_connect(void);

#endif /* USB_GAMEPAD_H */
