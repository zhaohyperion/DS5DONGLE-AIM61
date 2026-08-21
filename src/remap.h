#pragma once
#include <stdbool.h>
#include <stdint.h>

/* Nineteen physical DualSense controls map to a 19-bit logical target mask.
 * A zero mask disables the source; one bit is a conventional remap; multiple
 * bits are an atomic synchronous combination. */
#define REMAP_BTN_COUNT 19
#define REMAP_DIGITAL_BTN_COUNT 15
#define REMAP_WIRE_VERSION 3
#define REMAP_TARGET_MASK ((1u << REMAP_BTN_COUNT) - 1u)

#define REMAP_BTN_SQUARE 0
#define REMAP_BTN_CROSS 1
#define REMAP_BTN_CIRCLE 2
#define REMAP_BTN_TRIANGLE 3
#define REMAP_BTN_L1 4
#define REMAP_BTN_R1 5
#define REMAP_BTN_L2 6
#define REMAP_BTN_R2 7
#define REMAP_BTN_CREATE 8
#define REMAP_BTN_OPTIONS 9
#define REMAP_BTN_L3 10
#define REMAP_BTN_R3 11
#define REMAP_BTN_PS 12
#define REMAP_BTN_TP_CLICK 13
#define REMAP_BTN_MUTE 14
#define REMAP_BTN_DPAD_UP 15
#define REMAP_BTN_DPAD_RIGHT 16
#define REMAP_BTN_DPAD_DOWN 17
#define REMAP_BTN_DPAD_LEFT 18

void remap_init(void);
void remap_load(void);
bool remap_save(void);
bool remap_set_masks(const uint32_t *masks, uint8_t count,
                     uint16_t expected_revision);
void remap_get_masks(uint32_t *masks, uint8_t count);
uint16_t remap_revision(void);
uint32_t remap_physical_mask(const uint8_t *payload);
uint32_t remap_logical_mask(const uint8_t *payload);
void remap_write_logical_mask(uint8_t *payload, uint32_t mask,
                              uint8_t l2_value, uint8_t r2_value);
void remap_reset(void);
void remap_apply(uint8_t *payload);
void remap_kbd_tick(const uint8_t *payload);
void remap_on_disconnect(void);
bool remap_has_kbd_targets(void);
