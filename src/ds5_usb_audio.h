#ifndef DS5_USB_AUDIO_H
#define DS5_USB_AUDIO_H

#include <stdint.h>
#include <stdbool.h>

#define USB_AUDIO_EP_OUT        0x01
#define USB_AUDIO_MIC_EP_IN     0x82
#define USB_AUDIO_INTF_CTRL     0   /* Audio Control interface */
#define USB_AUDIO_INTF_STREAM   1   /* Audio Streaming OUT (speaker) */
#define USB_AUDIO_INTF_MIC      2   /* Audio Streaming IN  (mic) */

/* Speaker: 4ch, 16-bit, 48kHz: max (48+1)*4*2 = 392 bytes per 1ms frame */
#define USB_AUDIO_OUT_MPS       392
#define USB_AUDIO_SAMPLE_RATE   48000
#define USB_AUDIO_CHANNELS      4
#define USB_AUDIO_BITS          16

/* Mic: 2ch (mono duplicated to stereo), 16-bit, 48kHz: (48+1)*2*2 = 196 */
#define USB_AUDIO_MIC_CHANNELS  2
#define USB_AUDIO_MIC_MPS       196

/* One producer block is 512 samples per channel (10.67 ms at 48 kHz).
 * audio_task processes one block at a time and still combines two encoded
 * frames into the DS5 0x39 double-frame report.  Four buffers keep the USB
 * ISR independent from the encoder without placing deadline-critical PCM in
 * PSRAM. */
#define USB_AUDIO_BLOCK_SAMPLES 512
#define USB_AUDIO_PCM_BLOCK_COUNT 4

/* Compatibility name for code that sizes one acquired producer block. */
#define USB_AUDIO_ACCUM_SAMPLES USB_AUDIO_BLOCK_SAMPLES

/* Mic ring buffer: 4 Opus frames of stereo samples (was 2, expanded for
 * more USB ISO IN jitter tolerance to prevent underflow/pop artifacts) */
#define USB_AUDIO_MIC_RING_SAMPLES (480 * 4)

/**
 * A PCM buffer acquired from the USB ISO producer.
 *
 * The buffer is owned exclusively by the caller until usb_audio_release()
 * is called.  It contains USB_AUDIO_BLOCK_SAMPLES interleaved 4-channel
 * samples in FL, FR, haptics-L, haptics-R order.
 */
typedef struct {
    const int16_t *samples;
    uint32_t generation;
    uint32_t sequence;
    uint8_t index;
} usb_audio_block_t;

/**
 * Runtime overload counters.  All counters are monotonic uint32_t values and
 * may wrap.  They are intended for low-rate diagnostics, not ISR logging.
 */
typedef struct {
    uint32_t pcm_blocks_queued;
    uint32_t pcm_blocks_dropped;
    uint32_t pcm_pool_starvations;
    uint32_t pcm_queue_overruns;
    uint32_t pcm_stale_blocks;
    uint32_t pcm_ready_high_water;
    uint32_t mic_ring_overruns;
    uint32_t mic_ring_underruns;
    uint32_t mic_ep_write_errors;
} usb_audio_stats_t;

/**
 * Register Audio Control + Audio Streaming OUT interfaces and endpoint.
 * Must be called BEFORE HID interface registration in usb_gamepad_init().
 */
void usb_audio_early_init(void);
void usb_audio_register(uint8_t busid);

/**
 * Bind the task that consumes completed speaker blocks and deferred USB-audio
 * events.  The handle is kept opaque here so users of this header do not need
 * to include FreeRTOS task definitions.
 */
void usb_audio_set_consumer_task(void *task_handle);

/** Process and log events/errors captured by CherryUSB callbacks. Task only. */
void usb_audio_process_deferred(void);

/**
 * Get the audio portion of the config descriptor.
 * Returns pointer and length for embedding in the composite config descriptor.
 */
const uint8_t *usb_audio_get_desc(uint16_t *len);

/**
 * Check if audio streaming is active (USB host opened the speaker interface).
 */
bool usb_audio_is_active(void);

/**
 * Wait for and acquire one completed 512-sample PCM buffer.
 * Called only from audio_task context.  Stale buffers from an earlier USB
 * stream generation are discarded internally.
 *
 * @param block       receives the buffer pointer and ownership token
 * @param timeout_ms  maximum time to wait for the first queue item
 */
bool usb_audio_acquire(usb_audio_block_t *block, uint32_t timeout_ms);

/** Release a block previously returned by usb_audio_acquire(). */
void usb_audio_release(const usb_audio_block_t *block);

/**
 * Check whether an acquired block still belongs to the active USB stream.
 * Use before committing encoded data because SET_INTERFACE may close or
 * reopen the stream while audio_task is processing a block.
 */
bool usb_audio_block_is_current(const usb_audio_block_t *block);

/**
 * Reset audio streaming state (stream_active, PCM buffers, spk_active).
 * Called on USB RESET to avoid stale flags suppressing 0x31 output.
 */
void usb_audio_stop(void);

/** Pause active ISO endpoints for USB suspend without forgetting alt=1. */
void usb_audio_suspend(void);

/** Restore ISO endpoints selected before suspend. Called from USB resume ISR. */
void usb_audio_resume(uint8_t busid);

/** Pause DS5 audio while flash is programmed, preserving host alt settings. */
void usb_audio_set_maintenance(bool active);

/** Copy the current low-rate diagnostic counters into @p stats. */
void usb_audio_get_stats(usb_audio_stats_t *stats);

/**
 * Write decoded stereo PCM from mic into USB ring buffer.
 * Called from audio_mic_task after Opus decode.
 * @param samples  pointer to stereo int16 samples
 * @param count    number of stereo sample pairs (480 for one Opus frame)
 */
void usb_audio_mic_write(const int16_t *samples, uint32_t count);

/**
 * Check if mic streaming is active (host opened mic interface alt=1).
 */
bool usb_audio_mic_is_active(void);

/**
 * Drop decoded samples after a Bluetooth epoch change while preserving the
 * host's current UAC alternate setting and ongoing silence cadence.
 */
void usb_audio_mic_flush(void);

/**
 * Reset mic streaming state (ring buffer, active flag).
 * Called together with usb_audio_stop() on disconnect.
 */
void usb_audio_mic_stop(void);

#endif /* DS5_USB_AUDIO_H */
