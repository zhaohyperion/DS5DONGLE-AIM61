#include "ds5_usb_audio.h"
#include "audio.h"
#include "state_mgr.h"
#include "usbd_core.h"
#include "usbd_audio.h"
#include "FreeRTOS.h"
#include "queue.h"
#include "task.h"
#include "compiler/compiler_ld.h"
#include <string.h>
#include "debug_log.h"

/* ---- UAC1 entity IDs ---- */
#define AUDIO_IT_SPK_ID     0x01
#define AUDIO_FU_SPK_ID     0x02
#define AUDIO_OT_SPK_ID     0x03
#define AUDIO_IT_MIC_ID     0x04
#define AUDIO_FU_MIC_ID     0x05
#define AUDIO_OT_MIC_ID     0x06

/* UAC bInterval has different units at full and high speed.  Both settings
 * below describe the DS5-required 1 ms packet cadence. */
#ifdef FORCE_FS_MODE
#define USB_AUDIO_ISO_INTERVAL 0x01
#else
#define USB_AUDIO_ISO_INTERVAL 0x04
#endif

/* ---- Audio descriptor (AC + AS_OUT speaker + AS_IN mic) ----
 * AC_Intf(9)
 * + AC_Header(10) + IT_spk(12) + FU_spk(12) + OT_spk(9) + IT_mic(12) + FU_mic(9) + OT_mic(9)
 * + AS_OUT_Alt0(9) + AS_OUT_Alt1(9) + AS_General(7) + FormatType(11) + EP(9) + CS_EP(7)
 * + AS_IN_Alt0(9) + AS_IN_Alt1(9) + AS_General(7) + FormatType(11) + EP(9) + CS_EP(7)
 * = 186 bytes total
 * No IAD — matches DS5Dongle default and real DualSense dongle. */
#define AUDIO_DESC_SIZE 186

static const uint8_t audio_desc[AUDIO_DESC_SIZE] = {
    /* ---- Audio Control Interface (Interface 0) ---- */
    0x09, 0x04,
    USB_AUDIO_INTF_CTRL,            /* bInterfaceNumber */
    0x00, 0x00,                     /* bAlternateSetting, bNumEndpoints */
    0x01, 0x01, 0x00,               /* Audio, AudioControl, none */
    0x00,                           /* iInterface */

    /* AC Header: bInCollection=2 (speaker + mic) */
    0x0A,                           /* bLength: 8 + 2 */
    0x24, 0x01,                     /* CS_INTERFACE, HEADER */
    0x00, 0x01,                     /* bcdADC: 1.00 */
    0x49, 0x00,                     /* wTotalLength: 73 (10+12+12+9+12+9+9) */
    0x02,                           /* bInCollection: 2 streaming interfaces */
    USB_AUDIO_INTF_STREAM,          /* baInterfaceNr(1): speaker */
    USB_AUDIO_INTF_MIC,             /* baInterfaceNr(2): mic */

    /* ---- Speaker topology (same as before) ---- */

    /* Input Terminal (ID 1): USB Streaming, 4ch */
    0x0C, 0x24, 0x02,
    AUDIO_IT_SPK_ID,                /* bTerminalID: 1 */
    0x01, 0x01,                     /* wTerminalType: USB Streaming */
    AUDIO_OT_MIC_ID,                /* bAssocTerminal: 6 (paired with USB OUT) */
    0x04,                           /* bNrChannels: 4 */
    0x33, 0x00,                     /* wChannelConfig: FL+FR+SL+SR */
    0x00, 0x00,                     /* iChannelNames, iTerminal */

    /* Feature Unit (ID 2): Master mute+volume */
    0x0C, 0x24, 0x06,
    AUDIO_FU_SPK_ID,                /* bUnitID: 2 */
    AUDIO_IT_SPK_ID,                /* bSourceID: 1 */
    0x01,                           /* bControlSize: 1 byte */
    0x03,                           /* bmaControls[0] master: Mute+Volume */
    0x00, 0x00, 0x00, 0x00,         /* bmaControls[1..4]: no per-ch */
    0x00,                           /* iFeature */

    /* Output Terminal (ID 3): Speaker */
    0x09, 0x24, 0x03,
    AUDIO_OT_SPK_ID,                /* bTerminalID: 3 */
    0x01, 0x03,                     /* wTerminalType: Speaker (0x0301) */
    AUDIO_IT_MIC_ID,                /* bAssocTerminal: 4 (paired with mic input) */
    AUDIO_FU_SPK_ID,                /* bSourceID: 2 */
    0x00,                           /* iTerminal */

    /* ---- Microphone topology ---- */

    /* Input Terminal (ID 4): Headset Mic, 2ch */
    0x0C, 0x24, 0x02,
    AUDIO_IT_MIC_ID,                /* bTerminalID: 4 */
    0x02, 0x04,                     /* wTerminalType: Headset (0x0402) */
    AUDIO_OT_SPK_ID,                /* bAssocTerminal: 3 (paired with speaker) */
    0x02,                           /* bNrChannels: 2 (mono dup'd to stereo) */
    0x03, 0x00,                     /* wChannelConfig: FL+FR */
    0x00, 0x00,                     /* iChannelNames, iTerminal */

    /* Feature Unit (ID 5): Master mute+volume, 2ch */
    0x09, 0x24, 0x06,
    AUDIO_FU_MIC_ID,                /* bUnitID: 5 */
    AUDIO_IT_MIC_ID,                /* bSourceID: 4 */
    0x01,                           /* bControlSize: 1 byte */
    0x03,                           /* bmaControls[0] master: Mute+Volume */
    0x00,                           /* bmaControls[1] ch1: none */
    0x00,                           /* iFeature */

    /* Output Terminal (ID 6): USB Streaming */
    0x09, 0x24, 0x03,
    AUDIO_OT_MIC_ID,                /* bTerminalID: 6 */
    0x01, 0x01,                     /* wTerminalType: USB Streaming (0x0101) */
    AUDIO_IT_SPK_ID,                /* bAssocTerminal: 1 (paired with USB IN) */
    AUDIO_FU_MIC_ID,                /* bSourceID: 5 */
    0x00,                           /* iTerminal */

    /* ---- Audio Streaming OUT Interface (Interface 1: Speaker) ---- */
    /* Alt 0: idle */
    0x09, 0x04,
    USB_AUDIO_INTF_STREAM,
    0x00, 0x00,                     /* bAlternateSetting, bNumEndpoints */
    0x01, 0x02, 0x00, 0x00,

    /* Alt 1: active (1 ISO OUT endpoint) */
    0x09, 0x04,
    USB_AUDIO_INTF_STREAM,
    0x01,                           /* bAlternateSetting: 1 */
    0x01,                           /* bNumEndpoints: 1 */
    0x01, 0x02, 0x00, 0x00,

    /* AS General */
    0x07, 0x24, 0x01,
    AUDIO_IT_SPK_ID,                /* bTerminalLink: 1 */
    0x01,                           /* bDelay: 1 frame */
    0x01, 0x00,                     /* wFormatTag: PCM */

    /* Format Type I: 4ch, 16-bit, 48kHz */
    0x0B, 0x24, 0x02,
    0x01, 0x04, 0x02, 0x10,
    0x01, 0x80, 0xBB, 0x00,         /* 48000 Hz */

    /* ISO OUT Endpoint */
    0x09, 0x05,
    USB_AUDIO_EP_OUT,
    0x09,                           /* bmAttributes: Isochronous, Adaptive */
    (USB_AUDIO_OUT_MPS & 0xFF),
    (USB_AUDIO_OUT_MPS >> 8),
    USB_AUDIO_ISO_INTERVAL,         /* 1 ms at the selected USB speed */
    0x00, 0x00,

    /* CS Endpoint: General */
    0x07, 0x25, 0x01,
    0x00, 0x00, 0x00, 0x00,

    /* ---- Audio Streaming IN Interface (Interface 2: Mic) ---- */
    /* Alt 0: idle */
    0x09, 0x04,
    USB_AUDIO_INTF_MIC,
    0x00, 0x00,                     /* bAlternateSetting, bNumEndpoints */
    0x01, 0x02, 0x00, 0x00,

    /* Alt 1: active (1 ISO IN endpoint) */
    0x09, 0x04,
    USB_AUDIO_INTF_MIC,
    0x01,                           /* bAlternateSetting: 1 */
    0x01,                           /* bNumEndpoints: 1 */
    0x01, 0x02, 0x00, 0x00,

    /* AS General */
    0x07, 0x24, 0x01,
    AUDIO_OT_MIC_ID,                /* bTerminalLink: 6 (OT → USB Streaming) */
    0x01,                           /* bDelay: 1 frame */
    0x01, 0x00,                     /* wFormatTag: PCM */

    /* Format Type I: 2ch, 16-bit, 48kHz */
    0x0B, 0x24, 0x02,
    0x01, 0x02, 0x02, 0x10,
    0x01, 0x80, 0xBB, 0x00,         /* 48000 Hz */

    /* ISO IN Endpoint */
    0x09, 0x05,
    USB_AUDIO_MIC_EP_IN,
    0x05,                           /* bmAttributes: Isochronous, Asynchronous */
    (USB_AUDIO_MIC_MPS & 0xFF),
    (USB_AUDIO_MIC_MPS >> 8),
    USB_AUDIO_ISO_INTERVAL,         /* 1 ms at the selected USB speed */
    0x00, 0x00,

    /* CS Endpoint: General */
    0x07, 0x25, 0x01,
    0x00, 0x00, 0x00, 0x00,
};

/* ---- Owned PCM block pool for ISO OUT accumulation ----
 *
 * Buffer state has one writer (the CherryUSB endpoint ISR) and one reader
 * (audio_task).  A completed-buffer queue transfers ownership from ISR to
 * task.  The task returns ownership by changing READING -> FREE.  BL618 is a
 * single-core target, so byte state transitions plus the queue's FreeRTOS
 * synchronization are sufficient; explicit barriers document/preserve the
 * publish and release ordering for the compiler and RISC-V memory model. */
#define PCM_BUF_SAMPLES       USB_AUDIO_BLOCK_SAMPLES
#define PCM_INVALID_INDEX     0xFFu

enum pcm_buffer_state {
    PCM_BUFFER_FREE = 0,
    PCM_BUFFER_WRITING,
    PCM_BUFFER_READY,
    PCM_BUFFER_READING,
};

typedef struct {
    uint32_t generation;
    uint32_t sequence;
    uint8_t index;
} pcm_ready_item_t;

typedef struct {
    volatile uint32_t pcm_blocks_queued;
    volatile uint32_t pcm_blocks_dropped;
    volatile uint32_t pcm_pool_starvations;
    volatile uint32_t pcm_queue_overruns;
    volatile uint32_t pcm_stale_blocks;
    volatile uint32_t pcm_ready_high_water;
    volatile uint32_t mic_ring_overruns;
    volatile uint32_t mic_ring_underruns;
    volatile uint32_t mic_ep_write_errors;
} usb_audio_stats_internal_t;

static __attribute__((aligned(8))) int16_t
    pcm_buf[USB_AUDIO_PCM_BLOCK_COUNT][PCM_BUF_SAMPLES * USB_AUDIO_CHANNELS];
static volatile uint8_t  pcm_state[USB_AUDIO_PCM_BLOCK_COUNT];
static volatile uint32_t pcm_write_pos;
static volatile uint8_t  pcm_write_idx = PCM_INVALID_INDEX;
static volatile uint32_t pcm_generation;
static volatile uint32_t pcm_sequence;

static QueueHandle_t pcm_ready_queue;
static StaticQueue_t pcm_ready_queue_buf;
static __attribute__((aligned(4))) uint8_t pcm_ready_queue_storage[
    USB_AUDIO_PCM_BLOCK_COUNT * sizeof(pcm_ready_item_t)];

static usb_audio_stats_internal_t audio_stats;

static TaskHandle_t audio_consumer_task;

enum audio_deferred_event {
    AUDIO_DEFER_SPK_OPEN       = 1u << 0,
    AUDIO_DEFER_SPK_CLOSE      = 1u << 1,
    AUDIO_DEFER_MIC_OPEN       = 1u << 2,
    AUDIO_DEFER_MIC_CLOSE      = 1u << 3,
    AUDIO_DEFER_MIC_WRITE_ERR  = 1u << 4,
    AUDIO_DEFER_MIC_VOLUME     = 1u << 5,
    AUDIO_DEFER_MIC_MUTE       = 1u << 6,
    AUDIO_DEFER_SPK_MUTE       = 1u << 7,
};

static volatile uint32_t deferred_events;
static volatile uint32_t deferred_mic_write_errors;
static volatile int deferred_mic_write_error;
static volatile int deferred_mic_volume_db;
static volatile bool deferred_mic_mute;
static volatile bool deferred_spk_mute;

static inline void pcm_memory_barrier(void)
{
    __sync_synchronize();
}

static inline void audio_notify_consumer_from_isr(BaseType_t *woken)
{
    TaskHandle_t task = audio_consumer_task;
    if (task)
        vTaskNotifyGiveFromISR(task, woken);
}

static void audio_defer_from_isr(uint32_t events)
{
    BaseType_t woken = pdFALSE;
    deferred_events |= events;
    audio_notify_consumer_from_isr(&woken);
    portYIELD_FROM_ISR(woken);
}

static volatile bool stream_active;
static volatile bool stream_requested;
static volatile bool audio_maintenance;
static volatile bool audio_activity_seen;
static volatile uint32_t audio_last_active_tick;

static uint8_t USB_NOCACHE_RAM_SECTION iso_rx_buf[USB_AUDIO_OUT_MPS];

/* ---- Entity table for CherryUSB audio class ---- */
static struct audio_entity_info entity_table[] = {
    { .bEntityId = AUDIO_FU_SPK_ID,
      .bDescriptorSubtype = AUDIO_CONTROL_FEATURE_UNIT,
      .ep = USB_AUDIO_EP_OUT },
    { .bEntityId = AUDIO_FU_MIC_ID,
      .bDescriptorSubtype = AUDIO_CONTROL_FEATURE_UNIT,
      .ep = USB_AUDIO_MIC_EP_IN },
};

static struct usbd_interface audio_intf_ctrl;
static struct usbd_interface audio_intf_stream;
static struct usbd_interface audio_intf_mic;
static struct usbd_endpoint  audio_out_ep;
static struct usbd_endpoint  audio_mic_ep;

/* ---- Mic ring buffer ---- */
#define MIC_RING_SIZE   USB_AUDIO_MIC_RING_SAMPLES
static int16_t mic_ring[MIC_RING_SIZE * USB_AUDIO_MIC_CHANNELS];
static volatile uint32_t mic_ring_wr = 0;
static volatile uint32_t mic_ring_rd = 0;
static volatile bool     mic_active  = false;
static volatile bool     mic_requested = false;
static volatile uint32_t mic_generation = 1;

static uint8_t USB_NOCACHE_RAM_SECTION iso_mic_tx_buf[USB_AUDIO_MIC_MPS];

/* The caller either runs in the USB ISR or holds a task critical section. */
static void mic_reset_generation_locked(bool active)
{
    mic_active = false;
    mic_generation++;
    if (mic_generation == 0)
        mic_generation = 1;
    mic_ring_wr = 0;
    mic_ring_rd = 0;
    pcm_memory_barrier();
    mic_active = active;
}

/* Called only from the USB callback/ISR side. */
static bool pcm_claim_free_buffer(void)
{
    for (uint8_t i = 0; i < USB_AUDIO_PCM_BLOCK_COUNT; i++) {
        if (pcm_state[i] == PCM_BUFFER_FREE) {
            pcm_state[i] = PCM_BUFFER_WRITING;
            pcm_write_idx = i;
            pcm_memory_barrier();
            return true;
        }
    }

    pcm_write_idx = PCM_INVALID_INDEX;
    audio_stats.pcm_pool_starvations++;
    return false;
}

/* Begin a new, independently sequenced USB stream.  READY and READING
 * buffers are deliberately not reclaimed here: their ownership remains with
 * the queue/consumer until audio_task observes the stale generation. */
static void pcm_start_generation(void)
{
    uint8_t old_writer = pcm_write_idx;
    pcm_write_idx = PCM_INVALID_INDEX;
    if (old_writer < USB_AUDIO_PCM_BLOCK_COUNT &&
        pcm_state[old_writer] == PCM_BUFFER_WRITING) {
        pcm_state[old_writer] = PCM_BUFFER_FREE;
    }

    pcm_generation++;
    if (pcm_generation == 0)
        pcm_generation = 1;
    pcm_sequence = 0;
    pcm_write_pos = 0;
    pcm_claim_free_buffer();
}

/* Invalidate the producer side without stealing a READY/READING buffer. */
static void pcm_stop_generation(void)
{
    uint8_t old_writer = pcm_write_idx;
    pcm_write_idx = PCM_INVALID_INDEX;
    if (old_writer < USB_AUDIO_PCM_BLOCK_COUNT &&
        pcm_state[old_writer] == PCM_BUFFER_WRITING) {
        pcm_state[old_writer] = PCM_BUFFER_FREE;
    }

    pcm_generation++;
    if (pcm_generation == 0)
        pcm_generation = 1;
    pcm_sequence = 0;
    pcm_write_pos = 0;
    pcm_memory_barrier();
}

/* ---- ISO OUT endpoint callback (ISR context) ---- */
ATTR_TCM_SECTION
static __attribute__((noinline)) void audio_ep_out_handler(
    uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)ep;

    if (audio_maintenance || nbytes == 0 || !stream_active) {
        if (stream_active)
            usbd_ep_start_read(busid, USB_AUDIO_EP_OUT, iso_rx_buf, sizeof(iso_rx_buf));
        return;
    }

    audio_last_active_tick = (uint32_t)xTaskGetTickCountFromISR();
    audio_activity_seen = true;
    uint32_t samples = nbytes / (USB_AUDIO_CHANNELS * sizeof(int16_t));
    const int16_t *src = (const int16_t *)iso_rx_buf;
    uint32_t consumed = 0; /* sample frames, not individual int16 values */
    BaseType_t woken = pdFALSE;

    while (consumed < samples) {
        uint32_t pos = pcm_write_pos;
        uint32_t space = PCM_BUF_SAMPLES - pos;
        uint32_t chunk = samples - consumed;
        if (chunk > space) chunk = space;

        uint8_t writer = pcm_write_idx;
        if (writer < USB_AUDIO_PCM_BLOCK_COUNT) {
            memcpy(&pcm_buf[writer][pos * USB_AUDIO_CHANNELS],
                   &src[consumed * USB_AUDIO_CHANNELS],
                   chunk * USB_AUDIO_CHANNELS * sizeof(int16_t));
        }
        pos += chunk;
        consumed += chunk;

        if (pos >= PCM_BUF_SAMPLES) {
            pcm_ready_item_t item = {
                .generation = pcm_generation,
                .sequence = pcm_sequence++,
                .index = writer,
            };

            pcm_write_pos = 0;
            pcm_write_idx = PCM_INVALID_INDEX;

            if (writer < USB_AUDIO_PCM_BLOCK_COUNT) {
                pcm_memory_barrier();
                pcm_state[writer] = PCM_BUFFER_READY;
                pcm_memory_barrier();

                if (pcm_ready_queue &&
                    xQueueSendFromISR(pcm_ready_queue, &item, &woken) == pdTRUE) {
                    audio_stats.pcm_blocks_queued++;
                    audio_notify_consumer_from_isr(&woken);
                    UBaseType_t depth = uxQueueMessagesWaitingFromISR(pcm_ready_queue);
                    if (depth > audio_stats.pcm_ready_high_water)
                        audio_stats.pcm_ready_high_water = depth;
                } else {
                    /* The task never received ownership, so the ISR may
                     * immediately return this buffer to the free pool. */
                    pcm_state[writer] = PCM_BUFFER_FREE;
                    audio_stats.pcm_blocks_dropped++;
                    audio_stats.pcm_queue_overruns++;
                }
            } else {
                /* No buffer was available for this complete 512-frame
                 * interval.  Drop the whole interval to preserve alignment. */
                audio_stats.pcm_blocks_dropped++;
            }

            pcm_claim_free_buffer();
        } else {
            pcm_write_pos = pos;
        }
    }

    usbd_ep_start_read(busid, USB_AUDIO_EP_OUT, iso_rx_buf, sizeof(iso_rx_buf));
    portYIELD_FROM_ISR(woken);
}

/* ---- Mic EP IN: feed next packet from ring buffer ---- */
ATTR_TCM_SECTION
static __attribute__((noinline)) void mic_send_next(uint8_t busid)
{
    uint32_t generation = mic_generation;
    uint32_t rd = mic_ring_rd;
    uint32_t wr = mic_ring_wr;
    /* Pair with the producer's publish barrier before mic_ring_wr changes. */
    pcm_memory_barrier();
    uint32_t avail = (wr >= rd) ? (wr - rd) : (MIC_RING_SIZE - rd + wr);
    uint32_t samples_per_pkt = 48;  /* 48 stereo pairs per 1ms frame */
    int16_t *tx = (int16_t *)iso_mic_tx_buf;
    uint32_t to_send = (avail >= samples_per_pkt) ? samples_per_pkt : avail;

    if (to_send < samples_per_pkt)
        audio_stats.mic_ring_underruns++;

    uint32_t first = to_send;
    if (first > MIC_RING_SIZE - rd)
        first = MIC_RING_SIZE - rd;
    memcpy(tx, &mic_ring[rd * USB_AUDIO_MIC_CHANNELS],
           first * USB_AUDIO_MIC_CHANNELS * sizeof(int16_t));
    if (to_send > first) {
        memcpy(&tx[first * USB_AUDIO_MIC_CHANNELS], mic_ring,
               (to_send - first) * USB_AUDIO_MIC_CHANNELS * sizeof(int16_t));
    }
    if (to_send < samples_per_pkt) {
        memset(&tx[to_send * USB_AUDIO_MIC_CHANNELS], 0,
               (samples_per_pkt - to_send) * USB_AUDIO_MIC_CHANNELS *
                   sizeof(int16_t));
    }
    /* Do not commit a read index into a ring that was reset/reopened while
     * this packet was being assembled. */
    if (mic_active && generation == mic_generation) {
        pcm_memory_barrier();
        uint32_t next_rd = rd + to_send;
        if (next_rd >= MIC_RING_SIZE)
            next_rd -= MIC_RING_SIZE;
        mic_ring_rd = next_rd;
    } else {
        memset(iso_mic_tx_buf, 0,
               samples_per_pkt * USB_AUDIO_MIC_CHANNELS * sizeof(int16_t));
    }

    int ret = usbd_ep_start_write(busid, USB_AUDIO_MIC_EP_IN,
                                  iso_mic_tx_buf, samples_per_pkt * 2 * sizeof(int16_t));
    if (ret < 0) {
        audio_stats.mic_ep_write_errors++;
        deferred_mic_write_errors++;
        deferred_mic_write_error = ret;
        audio_defer_from_isr(AUDIO_DEFER_MIC_WRITE_ERR);
    }
}

ATTR_TCM_SECTION
static __attribute__((noinline)) void audio_mic_ep_in_handler(
    uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)ep; (void)nbytes;
    if (mic_active && !audio_maintenance)
        mic_send_next(busid);
}

/* ---- CherryUSB weak callback overrides ---- */

void usbd_audio_open(uint8_t busid, uint8_t intf)
{
    if (intf == USB_AUDIO_INTF_STREAM) {
        stream_requested = true;
        if (audio_maintenance)
            return;
        stream_active = true;
        pcm_start_generation();
        state_mgr_set_spk_active(true);
        usbd_ep_start_read(busid, USB_AUDIO_EP_OUT, iso_rx_buf, sizeof(iso_rx_buf));
        audio_defer_from_isr(AUDIO_DEFER_SPK_OPEN);
    } else if (intf == USB_AUDIO_INTF_MIC) {
        mic_requested = true;
        if (audio_maintenance)
            return;
        mic_reset_generation_locked(true);
        audio_set_mic_active(true);
        mic_send_next(busid);
        audio_defer_from_isr(AUDIO_DEFER_MIC_OPEN);
    }
}

void usbd_audio_close(uint8_t busid, uint8_t intf)
{
    if (intf == USB_AUDIO_INTF_STREAM) {
        stream_requested = false;
        usbd_ep_close(busid, USB_AUDIO_EP_OUT);
        stream_active = false;
        pcm_stop_generation();
        audio_reset_encoder();
        state_mgr_set_spk_active(false);
        audio_defer_from_isr(AUDIO_DEFER_SPK_CLOSE);
    } else if (intf == USB_AUDIO_INTF_MIC) {
        mic_requested = false;
        usbd_ep_close(busid, USB_AUDIO_MIC_EP_IN);
        mic_reset_generation_locked(false);
        audio_set_mic_active(false);
        audio_defer_from_isr(AUDIO_DEFER_MIC_CLOSE);
    }
}

void usbd_audio_set_volume(uint8_t busid, uint8_t ep, uint8_t ch, int volume_db)
{
    (void)busid; (void)ch;
    if (ep == USB_AUDIO_MIC_EP_IN) {
        deferred_mic_volume_db = volume_db;
        audio_defer_from_isr(AUDIO_DEFER_MIC_VOLUME);
        return;
    }
    /* Map Windows dB range [-100, 0] to DualSense [0, 127].
     * Previous mapping (vol = volume_db + 100) capped at 100/127 = 79%.
     * Correct mapping: -100dB → 0, 0dB → 127. */
    int vol = (int)((float)(volume_db + 100) * 127.0f / 100.0f + 0.5f);
    if (vol < 0)   vol = 0;
    if (vol > 127) vol = 127;
    state_mgr_set_volume((uint8_t)vol, (uint8_t)vol);
}

void usbd_audio_set_mute(uint8_t busid, uint8_t ep, uint8_t ch, bool mute)
{
    (void)busid; (void)ch;
    if (ep == USB_AUDIO_MIC_EP_IN) {
        deferred_mic_mute = mute;
        audio_defer_from_isr(AUDIO_DEFER_MIC_MUTE);
        return;
    }
    state_mgr_set_mute(mute);
    deferred_spk_mute = mute;
    audio_defer_from_isr(AUDIO_DEFER_SPK_MUTE);
}

uint32_t usbd_audio_get_sampling_freq(uint8_t busid, uint8_t ep)
{
    (void)busid; (void)ep;
    return USB_AUDIO_SAMPLE_RATE;
}

/* ---- Public API ---- */

void usb_audio_early_init(void)
{
    if (!pcm_ready_queue) {
        memset((void *)pcm_state, PCM_BUFFER_FREE, sizeof(pcm_state));
        memset((void *)&audio_stats, 0, sizeof(audio_stats));
        deferred_events = 0;
        deferred_mic_write_errors = 0;
        deferred_mic_write_error = 0;
        stream_active = false;
        stream_requested = false;
        audio_activity_seen = false;
        audio_last_active_tick = 0;
        pcm_generation = 1;
        pcm_write_idx = PCM_INVALID_INDEX;
        pcm_write_pos = 0;
        pcm_sequence = 0;
        mic_generation = 1;
        mic_ring_wr = 0;
        mic_ring_rd = 0;
        mic_active = false;
        mic_requested = false;
        pcm_ready_queue = xQueueCreateStatic(
            USB_AUDIO_PCM_BLOCK_COUNT,
            sizeof(pcm_ready_item_t),
            pcm_ready_queue_storage,
            &pcm_ready_queue_buf);
    }
}

void usb_audio_set_consumer_task(void *task_handle)
{
    taskENTER_CRITICAL();
    audio_consumer_task = (TaskHandle_t)task_handle;
    taskEXIT_CRITICAL();
}

void usb_audio_process_deferred(void)
{
    uint32_t events;
    uint32_t mic_write_errors;
    int mic_write_error;
    int mic_volume_db;
    bool mic_mute;
    bool spk_mute;

    taskENTER_CRITICAL();
    events = deferred_events;
    deferred_events = 0;
    mic_write_errors = deferred_mic_write_errors;
    deferred_mic_write_errors = 0;
    mic_write_error = deferred_mic_write_error;
    mic_volume_db = deferred_mic_volume_db;
    mic_mute = deferred_mic_mute;
    spk_mute = deferred_spk_mute;
    taskEXIT_CRITICAL();

    if (events & AUDIO_DEFER_SPK_OPEN)
        LOG_INF("[AUDIO] Speaker stream opened\n");
    if (events & AUDIO_DEFER_SPK_CLOSE)
        LOG_INF("[AUDIO] Speaker stream closed\n");
    if (events & AUDIO_DEFER_MIC_OPEN)
        LOG_INF("[AUDIO] Mic stream opened\n");
    if (events & AUDIO_DEFER_MIC_CLOSE)
        LOG_INF("[AUDIO] Mic stream closed\n");
    if (events & AUDIO_DEFER_MIC_VOLUME)
        LOG_INF("[AUDIO] Mic volume %d dB (ignored)\n", mic_volume_db);
    if (events & AUDIO_DEFER_MIC_MUTE)
        LOG_INF("[AUDIO] Mic mute = %d (ignored)\n", mic_mute);
    if (events & AUDIO_DEFER_SPK_MUTE)
        LOG_INF("[AUDIO] Mute = %d\n", spk_mute);
    if ((events & AUDIO_DEFER_MIC_WRITE_ERR) && mic_write_errors) {
        LOG_ERR("[AUDIO] mic EP write failures=%lu, last=%d\n",
                (unsigned long)mic_write_errors, mic_write_error);
    }
}

void usb_audio_register(uint8_t busid)
{
    usb_audio_early_init();

    usbd_add_interface(busid, usbd_audio_init_intf(
        busid, &audio_intf_ctrl, 0x0100, entity_table,
        sizeof(entity_table) / sizeof(entity_table[0])));
    usbd_add_interface(busid, usbd_audio_init_intf(
        busid, &audio_intf_stream, 0x0100, entity_table,
        sizeof(entity_table) / sizeof(entity_table[0])));
    usbd_add_interface(busid, usbd_audio_init_intf(
        busid, &audio_intf_mic, 0x0100, entity_table,
        sizeof(entity_table) / sizeof(entity_table[0])));

    audio_out_ep.ep_addr = USB_AUDIO_EP_OUT;
    audio_out_ep.ep_cb = audio_ep_out_handler;
    usbd_add_endpoint(busid, &audio_out_ep);

    audio_mic_ep.ep_addr = USB_AUDIO_MIC_EP_IN;
    audio_mic_ep.ep_cb = audio_mic_ep_in_handler;
    usbd_add_endpoint(busid, &audio_mic_ep);

    LOG_INF("[AUDIO] UAC1 interfaces registered (spk+mic)\n");
}

const uint8_t *usb_audio_get_desc(uint16_t *len)
{
    if (len) *len = AUDIO_DESC_SIZE;
    return audio_desc;
}

bool usb_audio_is_active(void)
{
    bool active;
    bool seen;
    uint32_t last_tick;

    taskENTER_CRITICAL();
    active = stream_active;
    seen = audio_activity_seen;
    last_tick = audio_last_active_tick;
    taskEXIT_CRITICAL();

    if (audio_maintenance)
        return false;
    if (active)
        return true;
    if (!seen)
        return false;
    uint32_t elapsed = (uint32_t)xTaskGetTickCount() - last_tick;
    return elapsed < (uint32_t)pdMS_TO_TICKS(5000);
}

void usb_audio_stop(void)
{
    if (xPortIsInsideInterrupt()) {
        stream_requested = false;
        stream_active = false;
        pcm_stop_generation();
    } else {
        taskENTER_CRITICAL();
        stream_requested = false;
        stream_active = false;
        pcm_stop_generation();
        taskEXIT_CRITICAL();
    }
    state_mgr_set_spk_active(false);
}

void usb_audio_suspend(void)
{
    /* Called by the CherryUSB event callback.  Preserve the host-selected
     * alternate settings, but invalidate every transport generation while the
     * BL618 USB engine is suspended. */
    stream_active = false;
    pcm_stop_generation();
    mic_reset_generation_locked(false);
    state_mgr_set_spk_active(false);
    audio_set_mic_active(false);
    audio_reset_encoder();
}

void usb_audio_resume(uint8_t busid)
{
    if (audio_maintenance)
        return;
    /* USB 2.0 does not require SET_INTERFACE to be repeated after resume.
     * Restart exactly the ISO endpoints that were selected before suspend. */
    if (stream_requested) {
        stream_active = true;
        pcm_start_generation();
        state_mgr_set_spk_active(true);
        usbd_ep_start_read(busid, USB_AUDIO_EP_OUT,
                           iso_rx_buf, sizeof(iso_rx_buf));
    }
    if (mic_requested) {
        mic_reset_generation_locked(true);
        audio_set_mic_active(true);
        mic_send_next(busid);
    }
}

void usb_audio_set_maintenance(bool active)
{
    if (active) {
        audio_maintenance = true;
        usb_audio_suspend();
    } else {
        audio_maintenance = false;
        usb_audio_resume(0);
    }
}

bool usb_audio_acquire(usb_audio_block_t *block, uint32_t timeout_ms)
{
    if (!block || !pcm_ready_queue)
        return false;

    pcm_ready_item_t item;
    TickType_t wait = pdMS_TO_TICKS(timeout_ms);

    /* Wait once, then drain stale generations without adding another full
     * timeout for every discarded queue item. */
    while (xQueueReceive(pcm_ready_queue, &item, wait) == pdTRUE) {
        wait = 0;

        if (item.index >= USB_AUDIO_PCM_BLOCK_COUNT)
            continue;

        pcm_memory_barrier();
        bool current = false;
        taskENTER_CRITICAL();
        if (pcm_state[item.index] == PCM_BUFFER_READY &&
            stream_active && item.generation == pcm_generation) {
            pcm_state[item.index] = PCM_BUFFER_READING;
            current = true;
        } else if (pcm_state[item.index] == PCM_BUFFER_READY) {
            pcm_state[item.index] = PCM_BUFFER_FREE;
        }
        taskEXIT_CRITICAL();

        if (!current) {
            audio_stats.pcm_stale_blocks++;
            continue;
        }

        pcm_memory_barrier();
        block->samples = pcm_buf[item.index];
        block->generation = item.generation;
        block->sequence = item.sequence;
        block->index = item.index;
        return true;
    }

    return false;
}

void usb_audio_release(const usb_audio_block_t *block)
{
    if (!block || block->index >= USB_AUDIO_PCM_BLOCK_COUNT)
        return;

    taskENTER_CRITICAL();
    if (pcm_state[block->index] == PCM_BUFFER_READING) {
        pcm_memory_barrier();
        pcm_state[block->index] = PCM_BUFFER_FREE;
        pcm_memory_barrier();
    }
    taskEXIT_CRITICAL();
}

bool usb_audio_block_is_current(const usb_audio_block_t *block)
{
    if (!block || block->index >= USB_AUDIO_PCM_BLOCK_COUNT)
        return false;

    bool current;
    taskENTER_CRITICAL();
    pcm_memory_barrier();
    current = stream_active &&
              block->generation == pcm_generation &&
              pcm_state[block->index] == PCM_BUFFER_READING;
    taskEXIT_CRITICAL();
    return current;
}

void usb_audio_get_stats(usb_audio_stats_t *stats)
{
    if (!stats)
        return;

    /* Each field is naturally aligned and atomically readable on RV32.  A
     * diagnostic snapshot may span one ISR update, which is acceptable for
     * monotonic telemetry and avoids masking the 1 ms USB interrupt. */
    stats->pcm_blocks_queued = audio_stats.pcm_blocks_queued;
    stats->pcm_blocks_dropped = audio_stats.pcm_blocks_dropped;
    stats->pcm_pool_starvations = audio_stats.pcm_pool_starvations;
    stats->pcm_queue_overruns = audio_stats.pcm_queue_overruns;
    stats->pcm_stale_blocks = audio_stats.pcm_stale_blocks;
    stats->pcm_ready_high_water = audio_stats.pcm_ready_high_water;
    stats->mic_ring_overruns = audio_stats.mic_ring_overruns;
    stats->mic_ring_underruns = audio_stats.mic_ring_underruns;
    stats->mic_ep_write_errors = audio_stats.mic_ep_write_errors;
}

/* ---- Mic ring buffer write (called from audio_mic_task) ---- */
void usb_audio_mic_write(const int16_t *samples, uint32_t count)
{
    if (audio_maintenance)
        return;
    if (!samples || count == 0)
        return;

    bool active;
    uint32_t generation;
    uint32_t wr;
    uint32_t rd;

    taskENTER_CRITICAL();
    active = mic_active;
    generation = mic_generation;
    wr = mic_ring_wr;
    rd = mic_ring_rd;
    taskEXIT_CRITICAL();

    if (!active)
        return;

    uint32_t used = (wr >= rd) ? (wr - rd) : (MIC_RING_SIZE - rd + wr);
    uint32_t free_samples = (MIC_RING_SIZE - 1u) - used;
    uint32_t to_write = count;
    if (to_write > free_samples) {
        to_write = free_samples;
        audio_stats.mic_ring_overruns++;
    }

    uint32_t first = to_write;
    if (first > MIC_RING_SIZE - wr)
        first = MIC_RING_SIZE - wr;
    memcpy(&mic_ring[wr * USB_AUDIO_MIC_CHANNELS], samples,
           first * USB_AUDIO_MIC_CHANNELS * sizeof(int16_t));
    if (to_write > first) {
        memcpy(mic_ring, &samples[first * USB_AUDIO_MIC_CHANNELS],
               (to_write - first) * USB_AUDIO_MIC_CHANNELS * sizeof(int16_t));
    }

    /* Publish the whole decoded frame (or its available prefix) at once.  A
     * close/reopen can reset the ring during the copies, so generation and
     * writer position must still match before the producer index is exposed. */
    pcm_memory_barrier();
    taskENTER_CRITICAL();
    if (mic_active && mic_generation == generation && mic_ring_wr == wr) {
        pcm_memory_barrier();
        uint32_t next_wr = wr + to_write;
        if (next_wr >= MIC_RING_SIZE)
            next_wr -= MIC_RING_SIZE;
        mic_ring_wr = next_wr;
    }
    taskEXIT_CRITICAL();
}

bool usb_audio_mic_is_active(void)
{
    return mic_active && !audio_maintenance;
}

void usb_audio_mic_flush(void)
{
    taskENTER_CRITICAL();
    bool active = mic_active;
    mic_reset_generation_locked(active);
    taskEXIT_CRITICAL();
}

void usb_audio_mic_stop(void)
{
    if (xPortIsInsideInterrupt()) {
        mic_requested = false;
        mic_reset_generation_locked(false);
    } else {
        taskENTER_CRITICAL();
        mic_requested = false;
        mic_reset_generation_locked(false);
        taskEXIT_CRITICAL();
    }
}
