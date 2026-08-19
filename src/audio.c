#include "audio.h"
#include "ds5_usb_audio.h"
#include "bt_hid_host.h"
#include "ds5_protocol.h"
#include "config.h"

#include "FreeRTOS.h"
#include "queue.h"
#include "task.h"
#include "bflb_mtimer.h"
#include "compiler/compiler_ld.h"

#include "opus.h"
#include <string.h>
#include "debug_log.h"
#include <math.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

/* ---- Constants matching DS5Dongle audio.cpp ---- */
#define OPUS_FRAME_SAMPLES  480     /* 10ms at 48kHz */
#define OPUS_OUT_SIZE       200     /* CBR output size */
#define HAPTIC_BUF_SIZE     64      /* 32 stereo int8 pairs */
#define HAPTIC_DECIMATE     16      /* 48kHz / 3kHz */
#define INPUT_FRAME_SAMPLES USB_AUDIO_BLOCK_SAMPLES  /* 512 */
/* Two USB producer blocks become two Opus frames,
 * each resampled 512→480 and encoded as one Opus frame. */

#define MIC_OPUS_SIZE       71      /* Opus encoded mic frame from DualSense */
#define MIC_CHANNELS        1       /* Controller sends mono mic */
#define MIC_QUEUE_DEPTH     4

/* Polyphase sinc resampler: 512→480 = 16:15 ratio
 * Matches DS5Dongle's WDL sinc resampler for anti-alias filtering.
 * gcd(512,480) = 32 → 15 unique phases, 8 taps per phase. */
#define SINC_HALF_TAPS  4
#define SINC_TAPS       (2 * SINC_HALF_TAPS)
#define RESAMP_PHASES   15
#define SINC_LEFT_PAD   (SINC_HALF_TAPS - 1)
#define SINC_RIGHT_PAD  (SINC_TAPS - SINC_HALF_TAPS - 1)

#define AUDIO_TIMING_BUCKET_US 250u
#define AUDIO_TIMING_BUCKETS   64u
#define AUDIO_BLOCK_DEADLINE_US 10667u /* 512 samples / 48 kHz */

typedef struct {
    uint64_t total_us;
    uint32_t samples;
    uint32_t last_us;
    uint32_t max_us;
    uint32_t histogram[AUDIO_TIMING_BUCKETS];
} audio_timing_accum_t;

typedef struct {
    uint32_t frames_processed;
    uint32_t pairs_submitted;
    uint32_t pairs_rejected;
    uint32_t discontinuities;
    uint32_t encode_errors;
    uint32_t pipeline_resets;
    uint32_t deadline_misses;
    audio_timing_accum_t block;
    audio_timing_accum_t resample;
    audio_timing_accum_t encode;
} audio_runtime_internal_t;

static uint8_t audio_seq;   /* sequence counter for 0x39 audio report */

/* Static buffers for Opus encoder/decoder — avoids 20+KB heap allocation.
 * Upper bounds verified at init via opus_encoder_get_size() / opus_decoder_get_size(). */
#define OPUS_ENC_MAX_SIZE 36864
#define OPUS_DEC_MAX_SIZE 24576
static __attribute__((aligned(8))) uint8_t encoder_mem[OPUS_ENC_MAX_SIZE];
static __attribute__((aligned(8))) uint8_t decoder_mem[OPUS_DEC_MAX_SIZE];
static OpusEncoder  *encoder;
static OpusDecoder  *decoder;
static uint8_t  packet_counter;
static uint8_t  mic_seq;
static volatile bool plug_headset;
static volatile bool mic_enabled;   /* host opened mic interface AND config allows */
static volatile bool mic_status_pending;  /* deferred: send 0x32 to controller */
static volatile uint32_t mic_status_generation;

static QueueHandle_t mic_queue;

static int16_t  spk_resamp[OPUS_FRAME_SAMPLES * 2];
static bool     mic_first_frame;
static volatile bool encoder_reset_pending;
static volatile bool speaker_transport_reset_pending;
static volatile bool speaker_counter_reset_pending;
static volatile bool decoder_reset_pending;
static TaskHandle_t audio_task_handle;
static uint8_t  frame_slot;
static bool     speaker_sequence_valid;
static uint32_t speaker_generation;
static uint32_t speaker_next_sequence;
static bool     pair_speaker_valid;
static bool     pair_speaker_enabled;
static audio_runtime_internal_t runtime_stats;

/* Double-frame buffers for 0x39 report (2x haptics + 2x opus per packet) */
static uint8_t  opus_slots[2][OPUS_OUT_SIZE];
static int8_t   haptic_slots[2][HAPTIC_BUF_SIZE];

/* Pre-computed polyphase sinc filter: [phase][tap] */
/* Coefficients are prepared with float once at boot, then consumed as Q15 in
 * the realtime path.  Keeping the table in integer form avoids 7,680 software
 * floating-point MACs for every stereo 512 -> 480 conversion. */
/* The writable lookup tables are rebuilt once at boot and then read on every
 * sample.  Keep them in DTCM: together they occupy only 1,680 bytes, while
 * placing them in pSRAM would add avoidable bus/cache jitter to the hot path. */
static ATTR_DTCM_SECTION int16_t sinc_coeff_q15[RESAMP_PHASES][SINC_TAPS];
static ATTR_DTCM_SECTION uint16_t sinc_center[OPUS_FRAME_SAMPLES];
static ATTR_DTCM_SECTION uint8_t sinc_phase[OPUS_FRAME_SAMPLES];

static void audio_timing_record(audio_timing_accum_t *timing,
                                uint32_t elapsed_us)
{
    uint32_t bucket = elapsed_us / AUDIO_TIMING_BUCKET_US;
    if (bucket >= AUDIO_TIMING_BUCKETS)
        bucket = AUDIO_TIMING_BUCKETS - 1;

    timing->samples++;
    timing->last_us = elapsed_us;
    timing->total_us += elapsed_us;
    if (elapsed_us > timing->max_us)
        timing->max_us = elapsed_us;
    timing->histogram[bucket]++;
}

static uint32_t audio_timing_p99_samples(const audio_timing_accum_t *timing,
                                         uint32_t samples)
{
    if (samples == 0)
        return 0;

    uint32_t target = samples - samples / 100u;
    uint32_t cumulative = 0;
    for (uint32_t i = 0; i < AUDIO_TIMING_BUCKETS; i++) {
        cumulative += timing->histogram[i];
        if (cumulative >= target)
            return (i + 1u) * AUDIO_TIMING_BUCKET_US;
    }
    return AUDIO_TIMING_BUCKETS * AUDIO_TIMING_BUCKET_US;
}

static uint32_t audio_timing_p99(const audio_timing_accum_t *timing)
{
    return audio_timing_p99_samples(timing, timing->samples);
}

static void resamp_sinc_init(void)
{
    const float cutoff = 480.0f / 512.0f; /* anti-alias at output Nyquist */

    for (int p = 0; p < RESAMP_PHASES; p++) {
        float frac = (float)p / (float)RESAMP_PHASES;
        float coeff_float[SINC_TAPS];
        float sum = 0.0f;

        for (int t = 0; t < SINC_TAPS; t++) {
            float x = (float)(t - (SINC_HALF_TAPS - 1)) - frac;

            /* sinc(x * cutoff) * cutoff */
            float sx = x * cutoff;
            float s;
            if (fabsf(sx) < 1e-6f)
                s = cutoff;
            else
                s = cutoff * sinf((float)M_PI * sx) / ((float)M_PI * sx);

            /* Hann window over kernel span */
            float wn = ((float)t + 0.5f) / (float)SINC_TAPS;
            float w = 0.5f * (1.0f - cosf(2.0f * (float)M_PI * wn));

            coeff_float[t] = s * w;
            sum += coeff_float[t];
        }

        if (fabsf(sum) > 1e-6f) {
            for (int t = 0; t < SINC_TAPS; t++)
                coeff_float[t] /= sum;
        }

        /* Quantise only after normalisation.  Apply the small rounding
         * residual to the largest-magnitude coefficient so the DC gain stays
         * as close as possible to unity without perturbing every tap. */
        int32_t q15_sum = 0;
        int largest = 0;
        float largest_abs = 0.0f;
        for (int t = 0; t < SINC_TAPS; t++) {
            float scaled = coeff_float[t] * 32768.0f;
            int32_t quantised = (int32_t)(scaled +
                (scaled >= 0.0f ? 0.5f : -0.5f));
            if (quantised > INT16_MAX)
                quantised = INT16_MAX;
            if (quantised < INT16_MIN)
                quantised = INT16_MIN;
            sinc_coeff_q15[p][t] = (int16_t)quantised;
            q15_sum += quantised;

            float magnitude = fabsf(coeff_float[t]);
            if (magnitude > largest_abs) {
                largest_abs = magnitude;
                largest = t;
            }
        }

        int32_t corrected = (int32_t)sinc_coeff_q15[p][largest] +
                            (32768 - q15_sum);
        if (corrected > INT16_MAX)
            corrected = INT16_MAX;
        if (corrected < INT16_MIN)
            corrected = INT16_MIN;
        sinc_coeff_q15[p][largest] = (int16_t)corrected;
    }

    /* Fixed 16:15 geometry: remove one hardware DIV/REM pair from every
     * output sample in the realtime loop. */
    for (uint32_t i = 0; i < OPUS_FRAME_SAMPLES; i++) {
        uint32_t src_pos = i * 16u;
        sinc_center[i] = (uint16_t)(src_pos / RESAMP_PHASES);
        sinc_phase[i] = (uint8_t)(src_pos % RESAMP_PHASES);
    }
}

/* ---- Polyphase sinc resample 512 → 480 (stereo int16, Q15) ---- */
ATTR_TCM_SECTION
static __attribute__((noinline)) void resample_512_480(
    const int16_t *in, int16_t *out)
{
    for (int i = 0; i < OPUS_FRAME_SAMPLES; i++) {
        int center = sinc_center[i];
        const int16_t *c = sinc_coeff_q15[sinc_phase[i]];
        const int16_t *s = &in[(center - SINC_LEFT_PAD) * 2];
        int32_t sum_l = 0, sum_r = 0;

#pragma GCC unroll 8
        for (int t = 0; t < SINC_TAPS; t++) {
            sum_l += (int32_t)s[t * 2]     * c[t];
            sum_r += (int32_t)s[t * 2 + 1] * c[t];
        }

        /* Symmetric rounding before the arithmetic shift keeps negative and
         * positive samples balanced around zero. */
        /* Arithmetic right shift rounds negative values toward -infinity.
         * Adding 0.5 LSB and subtracting one only for negative accumulators
         * implements symmetric nearest rounding without a one-count DC bias. */
        int32_t l = (sum_l + 16384 - (sum_l < 0)) >> 15;
        int32_t r = (sum_r + 16384 - (sum_r < 0)) >> 15;
        if (l > 32767) l = 32767; if (l < -32768) l = -32768;
        if (r > 32767) r = 32767; if (r < -32768) r = -32768;

        out[i * 2]     = (int16_t)l;
        out[i * 2 + 1] = (int16_t)r;
    }
}

/* ---- Haptics: 16:1 point-sample decimation (stereo int16 → stereo int8) ----
 * No low-pass filter — matches wired DS5 behavior (internal haptics also
 * run at 3 kHz without filtering). */
static void decimate_haptics(const int16_t *in, int8_t *out, uint32_t in_samples)
{
    uint32_t out_pairs = in_samples / HAPTIC_DECIMATE;
    if (out_pairs > HAPTIC_BUF_SIZE / 2)
        out_pairs = HAPTIC_BUF_SIZE / 2;

    /* haptics_gain [1.0,2.0] → fixed-point 8.8: 256..512.  Configuration
     * changes are rare, so avoid repeating the float conversion per block. */
    static float cached_gain_f = -1.0f;
    static int32_t cached_gain_fp = 256;
    float gain_f = config_get()->haptics_gain;
    if (gain_f != cached_gain_f) {
        cached_gain_f = gain_f;
        if (gain_f < 1.0f) gain_f = 1.0f;
        if (gain_f > 2.0f) gain_f = 2.0f;
        cached_gain_fp = (int32_t)(gain_f * 256.0f);
    }
    int32_t gain_fp = cached_gain_fp;

    for (uint32_t i = 0; i < out_pairs; i++) {
        uint32_t idx = (i * HAPTIC_DECIMATE) * 2;
        int32_t val_l = in[idx];
        int32_t val_r = in[idx + 1];
        val_l = (val_l * gain_fp) >> 16;
        val_r = (val_r * gain_fp) >> 16;
        if (val_l > 127) val_l = 127;
        if (val_l < -128) val_l = -128;
        if (val_r > 127) val_r = 127;
        if (val_r < -128) val_r = -128;
        out[i * 2]     = (int8_t)val_l;
        out[i * 2 + 1] = (int8_t)val_r;
    }
}

/* ---- Build and send BT report 0x39 (547 bytes, double-frame) ---- */
static int send_audio_report(bool speaker_enabled)
{
    static uint8_t pkt[DS5_BT_AUDIO_REPORT_SIZE];
    uint8_t next_audio_seq = (audio_seq + 1u) & 0x0Fu;
    uint8_t next_packet_counter = (uint8_t)(packet_counter + 2u);
    memset(pkt, 0, sizeof(pkt));

    pkt[0] = DS5_BT_AUDIO_REPORT_ID;
    pkt[1] = (audio_seq & 0x0F) << 4;

    /* Audio control header (tag 0x91, 6 fields) */
    pkt[2] = DS5_AUDIO_TAG_HEADER;
    pkt[3] = 6;
    pkt[4] = mic_enabled ? 0x7F : 0x7E;

    uint8_t buf_len = config_audio_buf_len();
    pkt[5] = buf_len;
    pkt[6] = buf_len;
    pkt[7] = buf_len;
    pkt[8] = buf_len;
    pkt[9] = next_packet_counter;

    /* Haptics (tag 0xD2, 2x 64-byte blocks) */
    pkt[10] = DS5_AUDIO_TAG_HAPTICS;
    pkt[11] = DS5_AUDIO_SAMPLE_SIZE;
    memcpy(pkt + 12, haptic_slots[0], HAPTIC_BUF_SIZE);
    memcpy(pkt + 12 + HAPTIC_BUF_SIZE, haptic_slots[1], HAPTIC_BUF_SIZE);

    /* Speaker Opus (tag 0xD3/0xD6, 2x 200-byte blocks) */
    if (speaker_enabled) {
        pkt[140] = plug_headset ? DS5_AUDIO_TAG_HEADSET : DS5_AUDIO_TAG_SPEAKER;
        pkt[141] = OPUS_OUT_SIZE;
        memcpy(pkt + 142, opus_slots[0], OPUS_OUT_SIZE);
        memcpy(pkt + 142 + OPUS_OUT_SIZE, opus_slots[1], OPUS_OUT_SIZE);
    }

    /* CRC32 */
    uint32_t crc = ds5_crc32(DS5_BT_OUTPUT_CRC_SEED, pkt,
                             DS5_BT_AUDIO_REPORT_SIZE - 4);
    ds5_write_le32(&pkt[DS5_BT_AUDIO_REPORT_SIZE - 4], crc);

    int ret = bt_hid_host_send_output(pkt, DS5_BT_AUDIO_REPORT_SIZE);
    if (ret == 0) {
        audio_seq = next_audio_seq;
        packet_counter = next_packet_counter;
    }
    return ret;
}

/* ---- Public API ---- */

int audio_init(void)
{
    resamp_sinc_init();

    int err;
    int enc_size = opus_encoder_get_size(2);
    int dec_size = opus_decoder_get_size(MIC_CHANNELS);

    if (enc_size > OPUS_ENC_MAX_SIZE) {
        LOG_ERR("[AUDIO] Opus encoder needs %d bytes, buffer is %d\n",
                enc_size, OPUS_ENC_MAX_SIZE);
        return -1;
    }
    if (dec_size > OPUS_DEC_MAX_SIZE) {
        LOG_ERR("[AUDIO] Opus decoder needs %d bytes, buffer is %d\n",
                dec_size, OPUS_DEC_MAX_SIZE);
        return -1;
    }

    encoder = (OpusEncoder *)encoder_mem;
    err = opus_encoder_init(encoder, 48000, 2,
                            OPUS_APPLICATION_RESTRICTED_LOWDELAY);
    if (err != OPUS_OK) {
        LOG_ERR("[AUDIO] Opus encoder init failed: %d\n", err);
        encoder = NULL;
        return -1;
    }

    opus_encoder_ctl(encoder, OPUS_SET_EXPERT_FRAME_DURATION(OPUS_FRAMESIZE_10_MS));
    opus_encoder_ctl(encoder, OPUS_SET_BITRATE(200 * 8 * 100));
    opus_encoder_ctl(encoder, OPUS_SET_VBR(0));
    opus_encoder_ctl(encoder, OPUS_SET_COMPLEXITY(0));

    decoder = (OpusDecoder *)decoder_mem;
    err = opus_decoder_init(decoder, 48000, MIC_CHANNELS);
    if (err != OPUS_OK) {
        LOG_ERR("[AUDIO] Opus decoder init failed: %d\n", err);
        decoder = NULL;
    }

    mic_queue = xQueueCreate(MIC_QUEUE_DEPTH, MIC_OPUS_SIZE);
    if (!mic_queue) {
        LOG_ERR("[AUDIO] mic_queue create failed\n");
    }

    if (!decoder || !mic_queue) {
        LOG_ERR("[AUDIO] Mic path unavailable (decoder=%p queue=%p)\n",
               (void *)decoder, (void *)mic_queue);
    }

    audio_seq = 0;
    packet_counter = 0;
    mic_seq = 0;
    plug_headset = false;
    mic_enabled = false;
    mic_status_pending = false;
    mic_status_generation = 1;
    mic_first_frame = false;
    encoder_reset_pending = false;
    speaker_transport_reset_pending = false;
    speaker_counter_reset_pending = false;
    decoder_reset_pending = false;
    audio_task_handle = NULL;
    frame_slot = 0;
    speaker_sequence_valid = false;
    speaker_generation = 0;
    speaker_next_sequence = 0;
    pair_speaker_valid = false;
    pair_speaker_enabled = false;
    memset(&runtime_stats, 0, sizeof(runtime_stats));
    memset(opus_slots, 0, sizeof(opus_slots));
    memset(haptic_slots, 0, sizeof(haptic_slots));

    LOG_INF("[AUDIO] Opus static init (enc=%d/%d dec=%d/%d mic_q=%p)\n",
           enc_size, OPUS_ENC_MAX_SIZE, dec_size, OPUS_DEC_MAX_SIZE,
           (void *)mic_queue);
    return 0;
}

/* ---- Send a 0x32 status report to toggle controller mic streaming ---- */
static int send_mic_status(bool enabled)
{
    if (bt_hid_host_get_state() != BT_HID_STATE_CONNECTED)
        return -1;

    uint8_t pkt[DS5_BT_OUTPUT_EXT_SIZE];
    memset(pkt, 0, sizeof(pkt));

    pkt[0] = DS5_BT_OUTPUT_REPORT_ID_EXT;
    pkt[1] = (mic_seq & 0x0F) << 4;

    pkt[2] = DS5_AUDIO_TAG_HEADER;
    pkt[3] = 1;
    pkt[4] = enabled ? 0x03 : 0x02;

    uint32_t crc = ds5_crc32(DS5_BT_OUTPUT_CRC_SEED, pkt,
                             DS5_BT_OUTPUT_EXT_SIZE - 4);
    ds5_write_le32(&pkt[DS5_BT_OUTPUT_EXT_SIZE - 4], crc);

    int ret = bt_hid_host_send_output(pkt, DS5_BT_OUTPUT_EXT_SIZE);
    if (ret == 0)
        mic_seq = (mic_seq + 1u) & 0x0Fu;
    return ret;
}

/* Must run only in audio_task context.  audio_reset_encoder() is also called
 * by CherryUSB callbacks, so it merely sets encoder_reset_pending. */
static void reset_speaker_pipeline_task(void)
{
    frame_slot = 0;
    speaker_sequence_valid = false;
    pair_speaker_valid = false;
    memset(opus_slots, 0, sizeof(opus_slots));
    memset(haptic_slots, 0, sizeof(haptic_slots));
    if (encoder)
        opus_encoder_ctl(encoder, OPUS_RESET_STATE);
    runtime_stats.pipeline_resets++;
}

static void service_speaker_reset_task(void)
{
    bool reset;
    bool drop_transport;
    bool reset_counters;

    /* Consume requests atomically.  A USB interrupt arriving after this
     * snapshot leaves a new request set for the next pass. */
    taskENTER_CRITICAL();
    reset = encoder_reset_pending;
    drop_transport = speaker_transport_reset_pending;
    reset_counters = speaker_counter_reset_pending;
    encoder_reset_pending = false;
    speaker_transport_reset_pending = false;
    speaker_counter_reset_pending = false;
    taskEXIT_CRITICAL();

    if (drop_transport)
        bt_hid_host_drop_audio_pending();
    if (reset_counters) {
        audio_seq = 0;
        packet_counter = 0;
    }
    if (reset)
        reset_speaker_pipeline_task();
}

static void reset_decoder_task(void)
{
    if (decoder)
        opus_decoder_ctl(decoder, OPUS_RESET_STATE);
    mic_first_frame = false;
}

/* Called only by audio_mic_task, so Opus decoder state has one owner. */
static void service_decoder_reset_task(void)
{
    bool reset;
    taskENTER_CRITICAL();
    reset = decoder_reset_pending;
    decoder_reset_pending = false;
    taskEXIT_CRITICAL();
    if (reset)
        reset_decoder_task();
}

static void notify_audio_task_from_context(void)
{
    TaskHandle_t task = audio_task_handle;
    if (!task)
        return;

    if (xPortIsInsideInterrupt()) {
        BaseType_t woken = pdFALSE;
        vTaskNotifyGiveFromISR(task, &woken);
        portYIELD_FROM_ISR(woken);
    } else {
        xTaskNotifyGive(task);
    }
}

static void service_mic_status_task(void)
{
    bool pending;
    bool enabled;
    uint32_t generation;

    taskENTER_CRITICAL();
    pending = mic_status_pending;
    enabled = mic_enabled;
    generation = mic_status_generation;
    taskEXIT_CRITICAL();

    if (!pending || bt_hid_host_get_state() != BT_HID_STATE_CONNECTED)
        return;

    if (send_mic_status(enabled) != 0)
        return;

    /* A state change during the send owns the next retry; only acknowledge
     * the exact state/generation that was successfully enqueued. */
    taskENTER_CRITICAL();
    if (mic_status_pending && mic_status_generation == generation)
        mic_status_pending = false;
    taskEXIT_CRITICAL();
}

void audio_task(void *arg)
{
    (void)arg;
    static int16_t spk_padded[
        (SINC_LEFT_PAD + INPUT_FRAME_SAMPLES + SINC_RIGHT_PAD) * 2];
    static int16_t hap_raw[INPUT_FRAME_SAMPLES * 2];

    audio_task_handle = xTaskGetCurrentTaskHandle();
    usb_audio_set_consumer_task(audio_task_handle);
    LOG_INF("[AUDIO] Task started (512-sample pipeline)\n");

    for (;;) {
        /* Consume any accumulated wake count before inspecting its flags and
         * queues.  A notification arriving afterwards remains pending. */
        (void)ulTaskNotifyTake(pdTRUE, 0);
        service_speaker_reset_task();
        usb_audio_process_deferred();
        service_mic_status_task();

        usb_audio_block_t block;
        if (!usb_audio_acquire(&block, 0)) {
            /* Reset/status/deferred-event notifications share this wakeup
             * with completed PCM blocks, eliminating the former 25 ms reset
             * latency while retaining a periodic retry for BT backpressure. */
            (void)ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(25));
            continue;
        }

        uint64_t block_started_us = bflb_mtimer_get_time_us();
        /* A close/reopen may have raced the queue handoff. */
        if (encoder_reset_pending)
            service_speaker_reset_task();

        bool connected =
            bt_hid_host_get_state() == BT_HID_STATE_CONNECTED;
        bool current = usb_audio_block_is_current(&block);

        if (connected && current) {
            /* A sequence gap means at least one complete 512-sample interval
             * was dropped. Never pair audio across it. */
            if (!speaker_sequence_valid) {
                speaker_generation = block.generation;
                speaker_next_sequence = block.sequence;
                speaker_sequence_valid = true;
            } else if (speaker_generation != block.generation ||
                       speaker_next_sequence != block.sequence) {
                runtime_stats.discontinuities++;
                reset_speaker_pipeline_task();
                speaker_generation = block.generation;
                speaker_next_sequence = block.sequence;
                speaker_sequence_valid = true;
            }

            bool speaker_enabled_now = !config_get()->disable_speaker;
            uint8_t slot = frame_slot;

            if (slot == 0) {
                pair_speaker_enabled = speaker_enabled_now;
                pair_speaker_valid = true;
            } else if (!pair_speaker_valid ||
                       pair_speaker_enabled != speaker_enabled_now) {
                /* Never combine one enabled frame and one disabled frame in
                 * the same 0x39. Discard slot 0 and rebase this block as the
                 * first frame under the new setting. */
                reset_speaker_pipeline_task();
                speaker_generation = block.generation;
                speaker_next_sequence = block.sequence;
                speaker_sequence_valid = true;
                slot = 0;
                pair_speaker_enabled = speaker_enabled_now;
                pair_speaker_valid = true;
            }

            const int16_t *pcm = block.samples;
            int16_t *spk_raw = &spk_padded[SINC_LEFT_PAD * 2];
            for (uint32_t i = 0; i < INPUT_FRAME_SAMPLES; i++) {
                spk_raw[i * 2]     = pcm[i * USB_AUDIO_CHANNELS];
                spk_raw[i * 2 + 1] = pcm[i * USB_AUDIO_CHANNELS + 1];
                hap_raw[i * 2]     = pcm[i * USB_AUDIO_CHANNELS + 2];
                hap_raw[i * 2 + 1] = pcm[i * USB_AUDIO_CHANNELS + 3];
            }

            /* Extend edge samples into the guard region used by the eight-tap
             * resampler, avoiding bounds checks in its TCM hot loop. */
            for (uint32_t i = 0; i < SINC_LEFT_PAD; i++) {
                spk_padded[i * 2] = spk_raw[0];
                spk_padded[i * 2 + 1] = spk_raw[1];
            }
            for (uint32_t i = 0; i < SINC_RIGHT_PAD; i++) {
                uint32_t dst = SINC_LEFT_PAD + INPUT_FRAME_SAMPLES + i;
                spk_padded[dst * 2] =
                    spk_raw[(INPUT_FRAME_SAMPLES - 1) * 2];
                spk_padded[dst * 2 + 1] =
                    spk_raw[(INPUT_FRAME_SAMPLES - 1) * 2 + 1];
            }

            decimate_haptics(hap_raw, haptic_slots[slot],
                              INPUT_FRAME_SAMPLES);

            bool frame_ok = true;
            if (pair_speaker_enabled) {
                uint64_t stage_started_us = bflb_mtimer_get_time_us();
                resample_512_480(spk_raw, spk_resamp);
                uint64_t stage_finished_us = bflb_mtimer_get_time_us();
                audio_timing_record(&runtime_stats.resample,
                    (uint32_t)(stage_finished_us - stage_started_us));

                stage_started_us = stage_finished_us;
                int encoded = opus_encode(encoder, spk_resamp,
                                          OPUS_FRAME_SAMPLES,
                                          opus_slots[slot], OPUS_OUT_SIZE);
                stage_finished_us = bflb_mtimer_get_time_us();
                audio_timing_record(&runtime_stats.encode,
                    (uint32_t)(stage_finished_us - stage_started_us));

                /* DS5 CBR packets are exactly 200 bytes. A short packet is
                 * not raw PCM and must never be padded into a fake frame. */
                if (encoded != OPUS_OUT_SIZE) {
                    runtime_stats.encode_errors++;
                    frame_ok = false;
                }
            } else {
                memset(opus_slots[slot], 0, OPUS_OUT_SIZE);
            }

            /* SET_INTERFACE or USB RESET may invalidate ownership while the
             * task was encoding. Discard rather than publish stale data. */
            current = usb_audio_block_is_current(&block) &&
                      !encoder_reset_pending;
            usb_audio_release(&block);
            current = current && !encoder_reset_pending;

            if (!frame_ok) {
                reset_speaker_pipeline_task();
            } else if (current &&
                       bt_hid_host_get_state() == BT_HID_STATE_CONNECTED) {
                runtime_stats.frames_processed++;
                speaker_next_sequence = block.sequence + 1u;
                if (slot == 1) {
                    if (send_audio_report(pair_speaker_enabled) == 0) {
                        runtime_stats.pairs_submitted++;
                        frame_slot = 0;
                        pair_speaker_valid = false;
                    } else {
                        runtime_stats.pairs_rejected++;
                        /* Sequence/counter were not committed. Opus state did
                         * advance, so restart prediction before another pair. */
                        reset_speaker_pipeline_task();
                    }
                } else {
                    frame_slot = 1;
                }
            } else {
                reset_speaker_pipeline_task();
            }
        } else {
            usb_audio_release(&block);
            if (speaker_sequence_valid || pair_speaker_valid)
                reset_speaker_pipeline_task();
        }

        uint32_t block_elapsed_us = (uint32_t)(
            bflb_mtimer_get_time_us() - block_started_us);
        audio_timing_record(&runtime_stats.block, block_elapsed_us);
        if (block_elapsed_us > AUDIO_BLOCK_DEADLINE_US)
            runtime_stats.deadline_misses++;
    }
}

void audio_set_headset(bool plugged)
{
    if (plugged != plug_headset)
        LOG_INF("[AUDIO] Headset %s\n", plugged ? "plugged" : "unplugged");
    plug_headset = plugged;
}

void audio_reset(void)
{
    plug_headset = false;

    taskENTER_CRITICAL();
    mic_enabled = false;
    mic_status_pending = false;
    mic_status_generation++;
    if (mic_status_generation == 0)
        mic_status_generation = 1;
    encoder_reset_pending = true;
    speaker_transport_reset_pending = true;
    speaker_counter_reset_pending = true;
    decoder_reset_pending = true;
    taskEXIT_CRITICAL();

    if (mic_queue)
        xQueueReset(mic_queue);
    usb_audio_mic_flush();

    /* audio_reset() is a BT task-context API. */
    if (audio_task_handle)
        xTaskNotifyGive(audio_task_handle);
}

void audio_reset_encoder(void)
{
    /* Do not call Opus or clear task-owned slots from USB callback context. */
    encoder_reset_pending = true;
    speaker_transport_reset_pending = true;
    notify_audio_task_from_context();
}

void audio_mic_feed(const uint8_t *opus_data, uint16_t len)
{
    if (!mic_enabled || !mic_queue) return;
    if (len < MIC_OPUS_SIZE) return;

    uint8_t frame[MIC_OPUS_SIZE];
    memcpy(frame, opus_data, MIC_OPUS_SIZE);
    if (xQueueSend(mic_queue, frame, 0) != pdTRUE) {
        uint8_t discard[MIC_OPUS_SIZE];
        xQueueReceive(mic_queue, discard, 0);
        xQueueSend(mic_queue, frame, 0);
    }
}

void audio_set_mic_active(bool active)
{
    bool enabled = active && !config_get()->disable_mic;
    bool in_isr = xPortIsInsideInterrupt();

    if (!in_isr)
        taskENTER_CRITICAL();
    bool changed = mic_enabled != enabled;
    mic_enabled = enabled;
    mic_status_generation++;
    if (mic_status_generation == 0)
        mic_status_generation = 1;
    mic_status_pending = true;
    if (changed || !enabled)
        decoder_reset_pending = true;
    if (!in_isr)
        taskEXIT_CRITICAL();

    notify_audio_task_from_context();
}

bool audio_mic_active(void)
{
    return mic_enabled;
}

static void audio_timing_export(const audio_timing_accum_t *source,
                                uint32_t *last, uint32_t *average,
                                uint32_t *maximum, uint32_t *p99)
{
    *last = source->last_us;
    *average = source->samples
        ? (uint32_t)(source->total_us / source->samples) : 0;
    *maximum = source->max_us;
    *p99 = audio_timing_p99(source);
}

void audio_get_runtime_stats(audio_runtime_stats_t *stats)
{
    if (!stats)
        return;

    audio_runtime_internal_t snapshot;
    taskENTER_CRITICAL();
    snapshot = runtime_stats;
    taskEXIT_CRITICAL();

    memset(stats, 0, sizeof(*stats));
    stats->frames_processed = snapshot.frames_processed;
    stats->pairs_submitted = snapshot.pairs_submitted;
    stats->pairs_rejected = snapshot.pairs_rejected;
    stats->discontinuities = snapshot.discontinuities;
    stats->encode_errors = snapshot.encode_errors;
    stats->pipeline_resets = snapshot.pipeline_resets;
    stats->deadline_misses = snapshot.deadline_misses;
    audio_timing_export(&snapshot.block,
                        &stats->block_us_last,
                        &stats->block_us_average,
                        &stats->block_us_max,
                        &stats->block_us_p99);
    audio_timing_export(&snapshot.resample,
                        &stats->resample_us_last,
                        &stats->resample_us_average,
                        &stats->resample_us_max,
                        &stats->resample_us_p99);
    audio_timing_export(&snapshot.encode,
                        &stats->encode_us_last,
                        &stats->encode_us_average,
                        &stats->encode_us_max,
                        &stats->encode_us_p99);
}

void audio_get_runtime_diag_stats(audio_runtime_diag_stats_t *stats)
{
    if (!stats)
        return;

    audio_runtime_diag_stats_t snapshot = {0};
    uint32_t block_samples;
    uint32_t resample_samples;
    uint32_t encode_samples;

    /* Keep interrupt masking bounded to thirteen aligned scalar loads.  The
     * large cumulative histograms are single-writer, monotonically increasing
     * data owned by audio_task.  Scanning them below may include the next
     * sample, which is acceptable for their bucketed low-rate p99 estimate and
     * avoids delaying USB, Bluetooth, or audio interrupts. */
    taskENTER_CRITICAL();
    snapshot.frames_processed = runtime_stats.frames_processed;
    snapshot.pairs_submitted = runtime_stats.pairs_submitted;
    snapshot.pairs_rejected = runtime_stats.pairs_rejected;
    snapshot.discontinuities = runtime_stats.discontinuities;
    snapshot.encode_errors = runtime_stats.encode_errors;
    snapshot.pipeline_resets = runtime_stats.pipeline_resets;
    snapshot.deadline_misses = runtime_stats.deadline_misses;
    block_samples = runtime_stats.block.samples;
    snapshot.block_us_max = runtime_stats.block.max_us;
    resample_samples = runtime_stats.resample.samples;
    snapshot.resample_us_max = runtime_stats.resample.max_us;
    encode_samples = runtime_stats.encode.samples;
    snapshot.encode_us_max = runtime_stats.encode.max_us;
    taskEXIT_CRITICAL();

    snapshot.block_us_p99 =
        audio_timing_p99_samples(&runtime_stats.block, block_samples);
    snapshot.resample_us_p99 =
        audio_timing_p99_samples(&runtime_stats.resample, resample_samples);
    snapshot.encode_us_p99 =
        audio_timing_p99_samples(&runtime_stats.encode, encode_samples);
    *stats = snapshot;
}

/* ---- Mic decode task: runs independently at lower priority than audio_task ----
 * Blocks on mic_queue so it doesn't burn CPU when mic is inactive.
 * Decodes one Opus frame per wakeup → writes to USB mic ring buffer.
 * Keeps audio_task cycle at ~21ms regardless of mic decoding cost. */
void audio_mic_task(void *arg)
{
    (void)arg;
    static uint8_t  mic_opus_buf[MIC_OPUS_SIZE];
    static int16_t  mic_mono[OPUS_FRAME_SAMPLES];
    static union {
        uint32_t packed[OPUS_FRAME_SAMPLES];
        int16_t stereo[OPUS_FRAME_SAMPLES * 2];
    } mic_output;

    for (;;) {
        service_decoder_reset_task();

        if (!mic_queue) {
            vTaskDelay(pdMS_TO_TICKS(10));
            continue;
        }

        if (xQueueReceive(mic_queue, mic_opus_buf, portMAX_DELAY) != pdTRUE)
            continue;

        service_decoder_reset_task();
        if (!decoder || !mic_enabled)
            continue;

        int decoded = opus_decode(decoder, mic_opus_buf, MIC_OPUS_SIZE,
                                  mic_mono, OPUS_FRAME_SAMPLES, 0);
        /* A close/reset during decode invalidates both decoder history and the
         * output. The mic task alone performs the actual decoder reset. */
        if (decoder_reset_pending || !mic_enabled) {
            service_decoder_reset_task();
            continue;
        }
        if (decoded <= 0) {
            LOG_ERR("[MIC] Opus decode error: %d\n", decoded);
            continue;
        }
        if (!mic_first_frame) {
            mic_first_frame = true;
            LOG_INF("[AUDIO] First mic frame decoded (%d samples)\n", decoded);
        }

        /* One aligned word store duplicates the mono sample into L/R.  The
         * union keeps the packed write defined for the GNU C target while
         * preserving the int16 API expected by the USB audio ring. */
        for (int i = 0; i < decoded; i++) {
            uint16_t sample = (uint16_t)mic_mono[i];
            mic_output.packed[i] = (uint32_t)sample |
                                   ((uint32_t)sample << 16);
        }
        usb_audio_mic_write(mic_output.stereo, (uint32_t)decoded);
    }
}
