#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct zb_engine zb_engine;

typedef enum zb_engine_state {
  ZB_STATE_IDLE = 0,
  ZB_STATE_OPENING = 1,
  ZB_STATE_BUFFERING = 2,
  ZB_STATE_READY = 3,
  ZB_STATE_PLAYING = 4,
  ZB_STATE_PAUSED = 5,
  ZB_STATE_ENDED = 6,
  ZB_STATE_ERROR = 7,
  ZB_STATE_RELEASED = 8,
} zb_engine_state;

typedef enum zb_engine_filter_type {
  ZB_FILTER_NONE = 0,
  ZB_FILTER_LOW_PASS = 1,
  ZB_FILTER_HIGH_PASS = 2,
} zb_engine_filter_type;

zb_engine *zb_engine_create(void);
void zb_engine_destroy(zb_engine *engine);

int32_t zb_engine_open_file(zb_engine *engine, const char *path);
int32_t zb_engine_open_url(zb_engine *engine, const char *url);
int32_t zb_engine_prebuffer_file(zb_engine *engine, const char *path);
int32_t zb_engine_prebuffer_url(zb_engine *engine, const char *url);
int32_t zb_engine_open_generated_tone(zb_engine *engine, int64_t duration_ms);
int32_t zb_engine_play(zb_engine *engine);
int32_t zb_engine_pause(zb_engine *engine);
int32_t zb_engine_stop(zb_engine *engine);
int32_t zb_engine_seek_ms(zb_engine *engine, int64_t position_ms);
int32_t zb_engine_set_volume(zb_engine *engine, float volume);
int32_t zb_engine_set_http_headers(zb_engine *engine, const char *headers,
                                   const char *user_agent);
int32_t zb_engine_set_filter(zb_engine *engine, int32_t enabled,
                             zb_engine_filter_type type, float cutoff_hz);

zb_engine_state zb_engine_get_state(zb_engine *engine);
int64_t zb_engine_get_position_ms(zb_engine *engine);
int64_t zb_engine_get_duration_ms(zb_engine *engine);
int64_t zb_engine_get_buffered_ms(zb_engine *engine);
int64_t zb_engine_get_decode_buffer_ms(zb_engine *engine);
int64_t zb_engine_get_ring_buffer_capacity_ms(zb_engine *engine);
int64_t zb_engine_get_ffmpeg_probe_size_bytes(zb_engine *engine);
int64_t zb_engine_get_ffmpeg_max_analyze_duration_us(zb_engine *engine);
int64_t zb_engine_get_underrun_count(zb_engine *engine);
int32_t zb_engine_get_spectrum(zb_engine *engine, uint8_t *bands,
                               int32_t band_count);
int32_t zb_engine_analyze_spectrum(const float *samples, int64_t sample_count,
                                   int32_t channels, uint8_t *bands,
                                   int32_t band_count);
const char *zb_engine_get_last_error(zb_engine *engine);
int32_t zb_engine_analyze_silence_file(const char *path,
                                       int64_t *leading_silence_ms,
                                       int64_t *trailing_silence_ms);

#ifdef __cplusplus
}
#endif
