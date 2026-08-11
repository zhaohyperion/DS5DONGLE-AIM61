#ifndef AUDIO_H
#define AUDIO_H

#include <stdint.h>
#include <stdbool.h>

/** Low-rate snapshot of the speaker realtime pipeline. */
typedef struct {
    uint32_t frames_processed;
    uint32_t pairs_submitted;
    uint32_t pairs_rejected;
    uint32_t discontinuities;
    uint32_t encode_errors;
    uint32_t pipeline_resets;
    uint32_t deadline_misses;
    uint32_t block_us_last;
    uint32_t block_us_average;
    uint32_t block_us_max;
    uint32_t block_us_p99;
    uint32_t resample_us_last;
    uint32_t resample_us_average;
    uint32_t resample_us_max;
    uint32_t resample_us_p99;
    uint32_t encode_us_last;
    uint32_t encode_us_average;
    uint32_t encode_us_max;
    uint32_t encode_us_p99;
} audio_runtime_stats_t;

/** Compact low-rate view used by the runtime diagnostic publisher. */
typedef struct {
    uint32_t frames_processed;
    uint32_t pairs_submitted;
    uint32_t pairs_rejected;
    uint32_t discontinuities;
    uint32_t encode_errors;
    uint32_t pipeline_resets;
    uint32_t deadline_misses;
    uint32_t block_us_max;
    uint32_t block_us_p99;
    uint32_t resample_us_max;
    uint32_t resample_us_p99;
    uint32_t encode_us_max;
    uint32_t encode_us_p99;
} audio_runtime_diag_stats_t;

/**
 * Initialize the audio processing pipeline (Opus encoder, buffers).
 * Must be called once from main before starting audio_task.
 * Returns 0 on success, -1 on Opus init failure.
 */
int audio_init(void);

/**
 * Audio processing task entry point (FreeRTOS).
 * Acquires owned 512-sample USB PCM buffers, processes one at a time,
 * then pairs two encoded frames in BT report 0x39 (double-frame).
 */
void audio_task(void *arg);

/**
 * Set headset plug state (parsed from BT input report byte[56] bit0).
 * When plugged, speaker tag switches from 0x93 to 0x96.
 */
void audio_set_headset(bool plugged);

/**
 * Reset audio state (called on controller disconnect).
 */
void audio_reset(void);

/**
 * Reset speaker encoder state (called on USB speaker stream close/open).
 * The reset is deferred to audio_task because this API may be called from a
 * CherryUSB callback/ISR. Clears Opus prediction and the pending frame pair.
 */
void audio_reset_encoder(void);

/**
 * Feed a mic Opus frame from the controller (called from BT callback context).
 * @param opus_data  pointer to the Opus-encoded frame
 * @param len        available byte length (must be >= 71)
 */
void audio_mic_feed(const uint8_t *opus_data, uint16_t len);

/**
 * Set mic active state (called when USB host opens/closes mic interface).
 * Sends a 0x32 status report to the controller to start/stop mic streaming.
 */
void audio_set_mic_active(bool active);

/**
 * Check if mic streaming is active.
 */
bool audio_mic_active(void);

/** Copy cumulative timing/overload counters for low-rate diagnostics. */
void audio_get_runtime_stats(audio_runtime_stats_t *stats);

/**
 * Copy only the counters exported by Feature Report 0xFD.  The critical
 * section copies scalar values only; histogram percentile estimation runs
 * afterwards in low-priority task context.
 */
void audio_get_runtime_diag_stats(audio_runtime_diag_stats_t *stats);

/**
 * Mic decode task entry point (FreeRTOS).
 * Blocks on mic_queue, decodes Opus frames, writes to USB mic ring buffer.
 * Must run at lower priority than audio_task to avoid blocking speaker path.
 */
void audio_mic_task(void *arg);

#endif /* AUDIO_H */
