#include "zb_audio_engine.h"

#include "third_party/miniaudio/miniaudio.h"

#include <curl/curl.h>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>
}

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cerrno>
#include <chrono>
#include <cmath>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

struct zb_biquad_filter {
  double b0_1 = 1.0;
  double b1_1 = 0.0;
  double b2_1 = 0.0;
  double a1_1 = 0.0;
  double a2_1 = 0.0;
  double b0_2 = 1.0;
  double b1_2 = 0.0;
  double b2_2 = 0.0;
  double a1_2 = 0.0;
  double a2_2 = 0.0;
  double s1_x1_l = 0.0;
  double s1_x2_l = 0.0;
  double s1_y1_l = 0.0;
  double s1_y2_l = 0.0;
  double s1_x1_r = 0.0;
  double s1_x2_r = 0.0;
  double s1_y1_r = 0.0;
  double s1_y2_r = 0.0;
  double s2_x1_l = 0.0;
  double s2_x2_l = 0.0;
  double s2_y1_l = 0.0;
  double s2_y2_l = 0.0;
  double s2_x1_r = 0.0;
  double s2_x2_r = 0.0;
  double s2_y1_r = 0.0;
  double s2_y2_r = 0.0;

  void reset() {
    s1_x1_l = 0.0;
    s1_x2_l = 0.0;
    s1_y1_l = 0.0;
    s1_y2_l = 0.0;
    s1_x1_r = 0.0;
    s1_x2_r = 0.0;
    s1_y1_r = 0.0;
    s1_y2_r = 0.0;
    s2_x1_l = 0.0;
    s2_x2_l = 0.0;
    s2_y1_l = 0.0;
    s2_y2_l = 0.0;
    s2_x1_r = 0.0;
    s2_x2_r = 0.0;
    s2_y1_r = 0.0;
    s2_y2_r = 0.0;
  }

  void update(float cutoff_hz, int sample_rate, zb_engine_filter_type type) {
    const auto clamped_cutoff =
        std::clamp<double>(cutoff_hz, 20.0, (sample_rate / 2.0) - 1.0);
    const auto omega =
        2.0 * 3.14159265358979323846 * clamped_cutoff / sample_rate;
    const auto sin_omega = std::sin(omega);
    const auto cos_omega = std::cos(omega);
    const auto alpha = sin_omega / (2.0 * 0.707);
    double raw_a0 = 1.0;

    if (type == ZB_FILTER_HIGH_PASS) {
      const auto v = (1.0 + cos_omega) / 2.0;
      b0_1 = v;
      b0_2 = v;
      b1_1 = -(1.0 + cos_omega);
      b1_2 = b1_1;
      b2_1 = v;
      b2_2 = v;
      raw_a0 = 1.0 + alpha;
      a1_1 = -2.0 * cos_omega;
      a1_2 = a1_1;
      a2_1 = 1.0 - alpha;
      a2_2 = a2_1;
    } else {
      const auto v = (1.0 - cos_omega) / 2.0;
      b0_1 = v;
      b0_2 = v;
      b1_1 = 1.0 - cos_omega;
      b1_2 = b1_1;
      b2_1 = v;
      b2_2 = v;
      raw_a0 = 1.0 + alpha;
      a1_1 = -2.0 * cos_omega;
      a1_2 = a1_1;
      a2_1 = 1.0 - alpha;
      a2_2 = a2_1;
    }

    b0_1 /= raw_a0;
    b1_1 /= raw_a0;
    b2_1 /= raw_a0;
    a1_1 /= raw_a0;
    a2_1 /= raw_a0;
    b0_2 /= raw_a0;
    b1_2 /= raw_a0;
    b2_2 /= raw_a0;
    a1_2 /= raw_a0;
    a2_2 /= raw_a0;
  }

  double process(double input, double &x1, double &x2, double &y1, double &y2,
                 int stage) {
    const auto b0 = stage == 1 ? b0_1 : b0_2;
    const auto b1 = stage == 1 ? b1_1 : b1_2;
    const auto b2 = stage == 1 ? b2_1 : b2_2;
    const auto a1 = stage == 1 ? a1_1 : a1_2;
    const auto a2 = stage == 1 ? a2_1 : a2_2;
    const auto output = b0 * input + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
    x2 = x1;
    x1 = input;
    y2 = y1;
    y1 = output;
    return output;
  }

  void process_stereo(float &left, float &right) {
    const auto mid_l = process(left, s1_x1_l, s1_x2_l, s1_y1_l, s1_y2_l, 1);
    const auto out_l = process(mid_l, s2_x1_l, s2_x2_l, s2_y1_l, s2_y2_l, 2);
    const auto mid_r = process(right, s1_x1_r, s1_x2_r, s1_y1_r, s1_y2_r, 1);
    const auto out_r = process(mid_r, s2_x1_r, s2_x2_r, s2_y1_r, s2_y2_r, 2);
    left = static_cast<float>(std::clamp(out_l, -1.0, 1.0));
    right = static_cast<float>(std::clamp(out_r, -1.0, 1.0));
  }
};

struct zb_engine {
  std::mutex control_mutex;
  std::mutex error_mutex;
  std::atomic<zb_engine_state> state{ZB_STATE_IDLE};
  std::atomic<int64_t> cursor_frames{0};
  std::atomic<int64_t> duration_frames{0};
  std::atomic<int64_t> duration_ms{0};
  std::atomic<int64_t> buffered_ms{0};
  std::atomic<float> volume{1.0f};
  std::atomic<bool> decode_stop{false};
  std::atomic<bool> decode_eof{false};
  std::atomic<bool> decode_failed{false};
  std::atomic<int64_t> decoded_frames{0};
  std::atomic<int64_t> underrun_count{0};
  std::atomic<int64_t> ring_buffer_capacity_frames{0};
  std::atomic<int64_t> ffmpeg_probe_size_bytes{0};
  std::atomic<int64_t> ffmpeg_max_analyze_duration_us{0};
  std::atomic<int32_t> filter_enabled{0};
  std::atomic<int32_t> filter_type{ZB_FILTER_NONE};
  std::atomic<float> filter_cutoff_hz{20000.0f};
  bool filter_was_enabled = false;
  int32_t filter_applied_type = ZB_FILTER_NONE;
  float filter_applied_cutoff_hz = 0.0f;
  zb_biquad_filter filter{};
  ma_device device{};
  ma_pcm_rb pcm_rb{};
  bool device_initialized = false;
  bool pcm_rb_initialized = false;
  bool using_ring_buffer = false;
  double phase = 0.0;
  double frequency_hz = 440.0;
  std::thread decode_thread;
  std::string source_uri;
  std::string http_headers;
  std::string http_user_agent;
  std::string last_error;
};

namespace {
constexpr int32_t ZB_OK = 0;
constexpr int32_t ZB_ERR_INVALID_ENGINE = -1;
constexpr int32_t ZB_ERR_INVALID_ARGUMENT = -2;
constexpr int32_t ZB_ERR_DEVICE = -3;
constexpr int32_t ZB_ERR_DECODE = -4;
constexpr int32_t ZB_SAMPLE_RATE = 48000;
constexpr int32_t ZB_CHANNELS = 2;
constexpr int32_t ZB_FILE_RING_BUFFER_FRAMES = ZB_SAMPLE_RATE;
constexpr int32_t ZB_URL_RING_BUFFER_FRAMES = ZB_SAMPLE_RATE * 3;
constexpr int32_t ZB_PREBUFFER_FRAMES = ZB_SAMPLE_RATE / 4;
constexpr int32_t ZB_OPEN_TIMEOUT_MS = 5000;
constexpr int32_t ZB_PREBUFFER_POLL_MS = 1;
constexpr int32_t ZB_RING_FULL_SLEEP_MS = 10;
constexpr int32_t ZB_SILENCE_WINDOW_MS = 100;
constexpr int32_t ZB_SILENCE_WINDOW_FRAMES =
    (ZB_SAMPLE_RATE * ZB_SILENCE_WINDOW_MS) / 1000;
constexpr double ZB_SILENCE_RMS_THRESHOLD_SQUARED = 0.00001;
constexpr int64_t ZB_FFMPEG_PROBE_SIZE_BYTES = 64 * 1024;
constexpr int64_t ZB_FFMPEG_MAX_ANALYZE_DURATION_US = 500 * 1000;
constexpr int32_t ZB_FFMPEG_MAX_PROBE_PACKETS = 32;
constexpr int64_t ZB_HTTP_RANGE_BYTES = 256 * 1024;
constexpr double ZB_TAU = 6.28318530717958647692;

struct http_range_input {
  std::string url;
  std::string headers;
  std::string user_agent;
  std::vector<uint8_t> chunk;
  int64_t position = 0;
  int64_t chunk_start = -1;
  int64_t total_size = -1;
  int64_t response_start = -1;
  std::string error;
  CURL *curl = nullptr;
  curl_slist *request_headers = nullptr;

  ~http_range_input() {
    curl_slist_free_all(request_headers);
    if (curl != nullptr) {
      curl_easy_cleanup(curl);
    }
  }
};

size_t append_http_body(char *data, size_t size, size_t count, void *opaque) {
  auto *input = static_cast<http_range_input *>(opaque);
  const auto bytes = size * count;
  const auto *begin = reinterpret_cast<uint8_t *>(data);
  input->chunk.insert(input->chunk.end(), begin, begin + bytes);
  return bytes;
}

size_t inspect_http_header(char *data, size_t size, size_t count,
                           void *opaque) {
  auto *input = static_cast<http_range_input *>(opaque);
  const auto bytes = size * count;
  std::string line(data, bytes);
  std::transform(line.begin(), line.end(), line.begin(), [](unsigned char ch) {
    return static_cast<char>(std::tolower(ch));
  });
  constexpr const char *prefix = "content-range: bytes ";
  if (line.rfind(prefix, 0) != 0) {
    return bytes;
  }
  const auto range_start = std::strlen(prefix);
  const auto dash = line.find('-', range_start);
  const auto slash =
      line.find('/', dash == std::string::npos ? range_start : dash + 1);
  if (dash == std::string::npos || slash == std::string::npos) {
    return bytes;
  }
  try {
    input->response_start =
        std::stoll(line.substr(range_start, dash - range_start));
    input->total_size = std::stoll(line.substr(slash + 1));
  } catch (const std::exception &) {
    input->response_start = -1;
  }
  return bytes;
}

curl_slist *parse_curl_headers(const std::string &serialized) {
  curl_slist *headers = nullptr;
  size_t offset = 0;
  while (offset < serialized.size()) {
    const auto end = serialized.find("\r\n", offset);
    const auto length =
        end == std::string::npos ? serialized.size() - offset : end - offset;
    if (length > 0) {
      headers =
          curl_slist_append(headers, serialized.substr(offset, length).c_str());
    }
    if (end == std::string::npos) {
      break;
    }
    offset = end + 2;
  }
  return headers;
}

bool fetch_http_range(http_range_input *input, int64_t start) {
  static std::once_flag curl_once;
  static CURLcode curl_init_result = CURLE_FAILED_INIT;
  std::call_once(curl_once, [] {
    curl_init_result = curl_global_init(CURL_GLOBAL_DEFAULT);
  });
  if (curl_init_result != CURLE_OK) {
    input->error = "libcurl initialization failed";
    return false;
  }

  if (input->curl == nullptr) {
    input->curl = curl_easy_init();
    if (input->curl == nullptr) {
      input->error = "libcurl handle allocation failed";
      return false;
    }
    input->request_headers = parse_curl_headers(input->headers);
    curl_easy_setopt(input->curl, CURLOPT_URL, input->url.c_str());
    curl_easy_setopt(input->curl, CURLOPT_HTTPHEADER, input->request_headers);
    curl_easy_setopt(input->curl, CURLOPT_USERAGENT, input->user_agent.c_str());
    curl_easy_setopt(input->curl, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(input->curl, CURLOPT_CONNECTTIMEOUT_MS, 5000L);
    curl_easy_setopt(input->curl, CURLOPT_TIMEOUT_MS, 15000L);
    curl_easy_setopt(input->curl, CURLOPT_NOSIGNAL, 1L);
    curl_easy_setopt(input->curl, CURLOPT_TCP_KEEPALIVE, 1L);
    curl_easy_setopt(input->curl, CURLOPT_WRITEFUNCTION, append_http_body);
    curl_easy_setopt(input->curl, CURLOPT_WRITEDATA, input);
    curl_easy_setopt(input->curl, CURLOPT_HEADERFUNCTION, inspect_http_header);
    curl_easy_setopt(input->curl, CURLOPT_HEADERDATA, input);
  }
  const auto end = start + ZB_HTTP_RANGE_BYTES - 1;
  const auto range = std::to_string(start) + "-" + std::to_string(end);
  input->chunk.clear();
  input->response_start = -1;
  input->error.clear();
  curl_easy_setopt(input->curl, CURLOPT_RANGE, range.c_str());
  const auto result = curl_easy_perform(input->curl);
  long status = 0;
  curl_easy_getinfo(input->curl, CURLINFO_RESPONSE_CODE, &status);

  if (result != CURLE_OK) {
    input->error = curl_easy_strerror(result);
    return false;
  }
  if (status != 206 && !(status == 200 && start == 0)) {
    input->error = "HTTP status " + std::to_string(status);
    return false;
  }
  if (input->response_start >= 0 && input->response_start != start) {
    input->error = "unexpected Content-Range offset";
    return false;
  }
  if (input->chunk.empty()) {
    input->error = "empty HTTP range response";
    return false;
  }
  if (status == 200 && input->total_size < 0) {
    input->total_size = static_cast<int64_t>(input->chunk.size());
  }
  input->chunk_start = start;
  return true;
}

int read_http_range(void *opaque, uint8_t *buffer, int buffer_size) {
  auto *input = static_cast<http_range_input *>(opaque);
  if (input->total_size >= 0 && input->position >= input->total_size) {
    return AVERROR_EOF;
  }
  int copied = 0;
  while (copied < buffer_size) {
    const auto chunk_end =
        input->chunk_start + static_cast<int64_t>(input->chunk.size());
    if (input->chunk_start < 0 || input->position < input->chunk_start ||
        input->position >= chunk_end) {
      if (!fetch_http_range(input, input->position)) {
        return copied > 0 ? copied : AVERROR(EIO);
      }
    }
    const auto chunk_offset = input->position - input->chunk_start;
    const auto available =
        static_cast<int64_t>(input->chunk.size()) - chunk_offset;
    auto wanted = static_cast<int64_t>(buffer_size - copied);
    if (input->total_size >= 0) {
      wanted = std::min(wanted, input->total_size - input->position);
    }
    const auto amount = std::min(available, wanted);
    if (amount <= 0) {
      break;
    }
    std::memcpy(buffer + copied, input->chunk.data() + chunk_offset,
                static_cast<size_t>(amount));
    copied += static_cast<int>(amount);
    input->position += amount;
  }
  return copied > 0 ? copied : AVERROR_EOF;
}

int64_t seek_http_range(void *opaque, int64_t offset, int whence) {
  auto *input = static_cast<http_range_input *>(opaque);
  if ((whence & AVSEEK_SIZE) != 0) {
    if (input->total_size < 0 && !fetch_http_range(input, input->position)) {
      return AVERROR(EIO);
    }
    return input->total_size;
  }
  whence &= ~AVSEEK_FORCE;
  int64_t target = offset;
  if (whence == SEEK_CUR) {
    target = input->position + offset;
  } else if (whence == SEEK_END) {
    if (input->total_size < 0 && !fetch_http_range(input, input->position)) {
      return AVERROR(EIO);
    }
    target = input->total_size + offset;
  } else if (whence != SEEK_SET) {
    return AVERROR(EINVAL);
  }
  if (target < 0 || (input->total_size >= 0 && target > input->total_size)) {
    return AVERROR(EINVAL);
  }
  input->position = target;
  return target;
}

int64_t frames_from_ms(int64_t value_ms) {
  return (value_ms * ZB_SAMPLE_RATE) / 1000;
}

int64_t ms_from_frames(int64_t frames) {
  return (frames * 1000) / ZB_SAMPLE_RATE;
}

std::string ffmpeg_error(int code) {
  char buffer[AV_ERROR_MAX_STRING_SIZE]{};
  av_strerror(code, buffer, sizeof(buffer));
  return buffer;
}

void clear_error(zb_engine *engine) {
  std::lock_guard<std::mutex> lock(engine->error_mutex);
  engine->last_error.clear();
  engine->decode_failed.store(false);
}

int32_t set_error(zb_engine *engine, const std::string &message, int32_t code) {
  if (engine != nullptr) {
    {
      std::lock_guard<std::mutex> lock(engine->error_mutex);
      engine->last_error = message;
    }
    engine->decode_failed.store(true);
    engine->state.store(ZB_STATE_ERROR);
  }
  return code;
}

int32_t set_error(zb_engine *engine, const char *message, int32_t code) {
  return set_error(engine, std::string(message), code);
}

bool is_released_or_null(zb_engine *engine) {
  return engine == nullptr || engine->state.load() == ZB_STATE_RELEASED;
}

int64_t read_duration_ms(AVFormatContext *format, AVStream *stream) {
  if (stream != nullptr && stream->duration != AV_NOPTS_VALUE) {
    return av_rescale_q(stream->duration, stream->time_base,
                        AVRational{1, 1000});
  }
  if (format != nullptr && format->duration != AV_NOPTS_VALUE) {
    return format->duration / 1000;
  }
  return 0;
}

AVFormatContext *allocate_format_context(zb_engine *engine) {
  auto *format = avformat_alloc_context();
  if (format == nullptr) {
    return nullptr;
  }
  format->probesize = ZB_FFMPEG_PROBE_SIZE_BYTES;
  format->max_analyze_duration = ZB_FFMPEG_MAX_ANALYZE_DURATION_US;
  format->max_probe_packets = ZB_FFMPEG_MAX_PROBE_PACKETS;
  format->flags |= AVFMT_FLAG_NOBUFFER;
  if (engine != nullptr) {
    engine->ffmpeg_probe_size_bytes.store(ZB_FFMPEG_PROBE_SIZE_BYTES);
    engine->ffmpeg_max_analyze_duration_us.store(
        ZB_FFMPEG_MAX_ANALYZE_DURATION_US);
  }
  return format;
}

void stop_decode_thread(zb_engine *engine) {
  if (engine == nullptr)
    return;
  engine->decode_stop.store(true);
  if (engine->decode_thread.joinable()) {
    engine->decode_thread.join();
  }
}

void uninit_ring_buffer(zb_engine *engine) {
  if (engine != nullptr && engine->pcm_rb_initialized) {
    ma_pcm_rb_uninit(&engine->pcm_rb);
    engine->pcm_rb_initialized = false;
  }
  if (engine != nullptr) {
    engine->using_ring_buffer = false;
    engine->ring_buffer_capacity_frames.store(0);
  }
}

void uninit_device(zb_engine *engine) {
  if (engine != nullptr && engine->device_initialized) {
    ma_device_stop(&engine->device);
    ma_device_uninit(&engine->device);
    engine->device_initialized = false;
  }
}

void reset_stream_locked(zb_engine *engine) {
  uninit_device(engine);
  stop_decode_thread(engine);
  uninit_ring_buffer(engine);
  engine->decode_stop.store(false);
  engine->decode_eof.store(false);
  engine->decode_failed.store(false);
  engine->decoded_frames.store(0);
  engine->buffered_ms.store(0);
  engine->cursor_frames.store(0);
  engine->duration_frames.store(0);
  engine->duration_ms.store(0);
  engine->ffmpeg_probe_size_bytes.store(0);
  engine->ffmpeg_max_analyze_duration_us.store(0);
  engine->filter_enabled.store(0);
  engine->filter_type.store(ZB_FILTER_NONE);
  engine->filter_cutoff_hz.store(20000.0f);
  engine->filter_was_enabled = false;
  engine->filter_applied_type = ZB_FILTER_NONE;
  engine->filter_applied_cutoff_hz = 0.0f;
  engine->filter.reset();
  engine->source_uri.clear();
}

int32_t init_output_device(zb_engine *engine);

int32_t init_ring_buffer(zb_engine *engine, int32_t capacity_frames) {
  const auto result =
      ma_pcm_rb_init(ma_format_f32, ZB_CHANNELS, capacity_frames, nullptr,
                     nullptr, &engine->pcm_rb);
  if (result != MA_SUCCESS) {
    return set_error(engine, "failed to initialize PCM ring buffer",
                     ZB_ERR_DEVICE);
  }
  ma_pcm_rb_set_sample_rate(&engine->pcm_rb, ZB_SAMPLE_RATE);
  engine->pcm_rb_initialized = true;
  engine->using_ring_buffer = true;
  engine->ring_buffer_capacity_frames.store(capacity_frames);
  return ZB_OK;
}

int32_t write_interleaved_to_ring(zb_engine *engine, const float *input,
                                  int64_t input_frames) {
  int64_t offset_frames = 0;
  while (offset_frames < input_frames && !engine->decode_stop.load()) {
    auto writable = static_cast<ma_uint32>(
        std::min<int64_t>(input_frames - offset_frames, 4096));
    void *write_buffer = nullptr;
    const auto acquire_result =
        ma_pcm_rb_acquire_write(&engine->pcm_rb, &writable, &write_buffer);
    if (acquire_result != MA_SUCCESS) {
      return set_error(engine, "ma_pcm_rb_acquire_write failed", ZB_ERR_DECODE);
    }
    if (writable == 0 || write_buffer == nullptr) {
      std::this_thread::sleep_for(
          std::chrono::milliseconds(ZB_RING_FULL_SLEEP_MS));
      continue;
    }

    const auto *source = input + (offset_frames * ZB_CHANNELS);
    std::memcpy(write_buffer, source, sizeof(float) * writable * ZB_CHANNELS);
    const auto commit_result =
        ma_pcm_rb_commit_write(&engine->pcm_rb, writable);
    if (commit_result != MA_SUCCESS) {
      return set_error(engine, "ma_pcm_rb_commit_write failed", ZB_ERR_DECODE);
    }

    offset_frames += writable;
    const auto absolute_decoded =
        engine->decoded_frames.fetch_add(writable) + writable;
    engine->buffered_ms.store(ms_from_frames(absolute_decoded));
  }
  return ZB_OK;
}

void close_format_input(AVFormatContext **format, AVIOContext **custom_io) {
  avformat_close_input(format);
  if (custom_io != nullptr && *custom_io != nullptr) {
    av_freep(&(*custom_io)->buffer);
    avio_context_free(custom_io);
  }
}

void cleanup_ffmpeg(AVPacket **packet, AVFrame **frame, SwrContext **swr,
                    AVChannelLayout *output_layout,
                    AVCodecContext **codec_context, AVFormatContext **format,
                    AVIOContext **custom_io = nullptr) {
  av_packet_free(packet);
  av_frame_free(frame);
  swr_free(swr);
  av_channel_layout_uninit(output_layout);
  avcodec_free_context(codec_context);
  close_format_input(format, custom_io);
}

int32_t analyze_silence_file(const char *path, int64_t *leading_silence_ms,
                             int64_t *trailing_silence_ms) {
  if (path == nullptr || leading_silence_ms == nullptr ||
      trailing_silence_ms == nullptr) {
    return ZB_ERR_INVALID_ARGUMENT;
  }
  *leading_silence_ms = 0;
  *trailing_silence_ms = 0;

  AVFormatContext *format = allocate_format_context(nullptr);
  if (format == nullptr)
    return ZB_ERR_DECODE;

  AVDictionary *open_options = nullptr;
  av_dict_set_int(&open_options, "probesize", ZB_FFMPEG_PROBE_SIZE_BYTES, 0);
  av_dict_set_int(&open_options, "analyzeduration",
                  ZB_FFMPEG_MAX_ANALYZE_DURATION_US, 0);
  auto result = avformat_open_input(&format, path, nullptr, &open_options);
  av_dict_free(&open_options);
  if (result < 0) {
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  result = avformat_find_stream_info(format, nullptr);
  if (result < 0) {
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  const auto stream_index =
      av_find_best_stream(format, AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);
  if (stream_index < 0) {
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  AVStream *stream = format->streams[stream_index];
  const AVCodec *codec = avcodec_find_decoder(stream->codecpar->codec_id);
  if (codec == nullptr) {
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  AVCodecContext *codec_context = avcodec_alloc_context3(codec);
  if (codec_context == nullptr) {
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  result = avcodec_parameters_to_context(codec_context, stream->codecpar);
  if (result < 0) {
    avcodec_free_context(&codec_context);
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  codec_context->thread_count = 1;
  codec_context->thread_type = 0;
  result = avcodec_open2(codec_context, codec, nullptr);
  if (result < 0) {
    avcodec_free_context(&codec_context);
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  AVChannelLayout output_layout{};
  av_channel_layout_default(&output_layout, ZB_CHANNELS);
  SwrContext *swr = nullptr;
  result = swr_alloc_set_opts2(&swr, &output_layout, AV_SAMPLE_FMT_FLT,
                               ZB_SAMPLE_RATE, &codec_context->ch_layout,
                               codec_context->sample_fmt,
                               codec_context->sample_rate, 0, nullptr);
  if (result < 0 || swr == nullptr || swr_init(swr) < 0) {
    swr_free(&swr);
    av_channel_layout_uninit(&output_layout);
    avcodec_free_context(&codec_context);
    avformat_close_input(&format);
    return ZB_ERR_DECODE;
  }

  AVPacket *packet = av_packet_alloc();
  AVFrame *frame = av_frame_alloc();
  if (packet == nullptr || frame == nullptr) {
    cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                   &format);
    return ZB_ERR_DECODE;
  }

  std::vector<double> window_sum_squares;
  std::vector<int64_t> window_sample_counts;
  std::vector<float> converted;
  int64_t total_frames = 0;

  auto append_converted = [&](const float *samples, int64_t frames) {
    for (int64_t frame = 0; frame < frames; ++frame) {
      const auto window_index =
          static_cast<size_t>(total_frames / ZB_SILENCE_WINDOW_FRAMES);
      if (window_sum_squares.size() <= window_index) {
        window_sum_squares.resize(window_index + 1, 0.0);
        window_sample_counts.resize(window_index + 1, 0);
      }
      for (int channel = 0; channel < ZB_CHANNELS; ++channel) {
        const double sample = samples[(frame * ZB_CHANNELS) + channel];
        window_sum_squares[window_index] += sample * sample;
        window_sample_counts[window_index] += 1;
      }
      total_frames += 1;
    }
  };

  auto receive_frames = [&]() -> int32_t {
    while (true) {
      const auto receive_result = avcodec_receive_frame(codec_context, frame);
      if (receive_result == AVERROR(EAGAIN) || receive_result == AVERROR_EOF)
        return ZB_OK;
      if (receive_result < 0)
        return ZB_ERR_DECODE;

      const auto out_samples = static_cast<int>(av_rescale_rnd(
          swr_get_delay(swr, codec_context->sample_rate) + frame->nb_samples,
          ZB_SAMPLE_RATE, codec_context->sample_rate, AV_ROUND_UP));
      const auto converted_size =
          static_cast<size_t>(out_samples * ZB_CHANNELS);
      if (converted.size() < converted_size)
        converted.resize(converted_size);
      uint8_t *output_data = reinterpret_cast<uint8_t *>(converted.data());
      const auto converted_samples =
          swr_convert(swr, &output_data, out_samples,
                      const_cast<const uint8_t **>(frame->extended_data),
                      frame->nb_samples);
      av_frame_unref(frame);
      if (converted_samples < 0)
        return ZB_ERR_DECODE;
      append_converted(converted.data(),
                       static_cast<int64_t>(converted_samples));
    }
  };

  while (av_read_frame(format, packet) >= 0) {
    if (packet->stream_index == stream_index) {
      result = avcodec_send_packet(codec_context, packet);
      av_packet_unref(packet);
      if (result < 0 || receive_frames() != ZB_OK) {
        cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                       &format);
        return ZB_ERR_DECODE;
      }
    } else {
      av_packet_unref(packet);
    }
  }

  avcodec_send_packet(codec_context, nullptr);
  if (receive_frames() != ZB_OK) {
    cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                   &format);
    return ZB_ERR_DECODE;
  }

  auto is_silent_window = [&](size_t index) -> bool {
    if (index >= window_sum_squares.size() || window_sample_counts[index] <= 0)
      return true;
    return (window_sum_squares[index] /
            static_cast<double>(window_sample_counts[index])) <=
           ZB_SILENCE_RMS_THRESHOLD_SQUARED;
  };

  size_t leading_windows = 0;
  while (leading_windows < window_sum_squares.size() &&
         is_silent_window(leading_windows)) {
    leading_windows += 1;
  }
  size_t trailing_windows = 0;
  while (trailing_windows < window_sum_squares.size() &&
         is_silent_window(window_sum_squares.size() - trailing_windows - 1)) {
    trailing_windows += 1;
  }

  *leading_silence_ms =
      static_cast<int64_t>(leading_windows) * ZB_SILENCE_WINDOW_MS;
  *trailing_silence_ms =
      static_cast<int64_t>(trailing_windows) * ZB_SILENCE_WINDOW_MS;

  cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                 &format);
  return ZB_OK;
}

void decode_file_worker(zb_engine *engine, std::string path, int64_t start_ms) {
  AVFormatContext *format = allocate_format_context(engine);
  if (format == nullptr) {
    set_error(engine, "failed to allocate FFmpeg format context",
              ZB_ERR_DECODE);
    return;
  }

  AVDictionary *open_options = nullptr;
  AVIOContext *custom_io = nullptr;
  http_range_input http_input;
  const auto is_http =
      path.rfind("http://", 0) == 0 || path.rfind("https://", 0) == 0;
  if (is_http) {
    http_input.url = path;
    http_input.headers = engine->http_headers;
    http_input.user_agent = engine->http_user_agent;
    auto *io_buffer = static_cast<uint8_t *>(av_malloc(64 * 1024));
    if (io_buffer == nullptr) {
      avformat_close_input(&format);
      set_error(engine, "failed to allocate HTTP input buffer", ZB_ERR_DECODE);
      return;
    }
    custom_io = avio_alloc_context(io_buffer, 64 * 1024, 0, &http_input,
                                   read_http_range, nullptr, seek_http_range);
    if (custom_io == nullptr) {
      av_free(io_buffer);
      avformat_close_input(&format);
      set_error(engine, "failed to allocate HTTP input context", ZB_ERR_DECODE);
      return;
    }
    custom_io->seekable = AVIO_SEEKABLE_NORMAL;
    format->pb = custom_io;
    format->flags |= AVFMT_FLAG_CUSTOM_IO;
  }
  av_dict_set_int(&open_options, "probesize", ZB_FFMPEG_PROBE_SIZE_BYTES, 0);
  av_dict_set_int(&open_options, "analyzeduration",
                  ZB_FFMPEG_MAX_ANALYZE_DURATION_US, 0);
  if (!is_http && !engine->http_headers.empty()) {
    av_dict_set(&open_options, "headers", engine->http_headers.c_str(), 0);
  }
  if (!is_http && !engine->http_user_agent.empty()) {
    av_dict_set(&open_options, "user_agent", engine->http_user_agent.c_str(),
                0);
  }

  auto result = avformat_open_input(&format, is_http ? nullptr : path.c_str(),
                                    nullptr, &open_options);
  av_dict_free(&open_options);
  if (result < 0) {
    close_format_input(&format, &custom_io);
    const auto detail =
        http_input.error.empty()
            ? ffmpeg_error(result)
            : http_input.error + " (" + ffmpeg_error(result) + ")";
    set_error(engine, "avformat_open_input failed: " + detail, ZB_ERR_DECODE);
    return;
  }

  result = avformat_find_stream_info(format, nullptr);
  if (result < 0) {
    close_format_input(&format, &custom_io);
    set_error(engine,
              "avformat_find_stream_info failed: " + ffmpeg_error(result),
              ZB_ERR_DECODE);
    return;
  }

  const auto stream_index =
      av_find_best_stream(format, AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);
  if (stream_index < 0) {
    close_format_input(&format, &custom_io);
    set_error(engine, "no audio stream found", ZB_ERR_DECODE);
    return;
  }

  AVStream *stream = format->streams[stream_index];
  const auto known_duration_ms = read_duration_ms(format, stream);
  if (known_duration_ms > 0) {
    engine->duration_ms.store(known_duration_ms);
    engine->duration_frames.store(frames_from_ms(known_duration_ms));
  }

  const AVCodec *codec = avcodec_find_decoder(stream->codecpar->codec_id);
  if (codec == nullptr) {
    close_format_input(&format, &custom_io);
    set_error(engine, "audio decoder not found", ZB_ERR_DECODE);
    return;
  }

  AVCodecContext *codec_context = avcodec_alloc_context3(codec);
  if (codec_context == nullptr) {
    close_format_input(&format, &custom_io);
    set_error(engine, "failed to allocate decoder context", ZB_ERR_DECODE);
    return;
  }

  result = avcodec_parameters_to_context(codec_context, stream->codecpar);
  if (result < 0) {
    avcodec_free_context(&codec_context);
    close_format_input(&format, &custom_io);
    set_error(engine,
              "avcodec_parameters_to_context failed: " + ffmpeg_error(result),
              ZB_ERR_DECODE);
    return;
  }

  codec_context->thread_count = 1;
  codec_context->thread_type = 0;

  result = avcodec_open2(codec_context, codec, nullptr);
  if (result < 0) {
    avcodec_free_context(&codec_context);
    close_format_input(&format, &custom_io);
    set_error(engine, "avcodec_open2 failed: " + ffmpeg_error(result),
              ZB_ERR_DECODE);
    return;
  }

  if (start_ms > 0) {
    const auto target_ts =
        av_rescale_q(start_ms, AVRational{1, 1000}, stream->time_base);
    avformat_seek_file(format, stream_index, INT64_MIN, target_ts, INT64_MAX,
                       AVSEEK_FLAG_BACKWARD);
    avcodec_flush_buffers(codec_context);
  }

  AVChannelLayout output_layout{};
  av_channel_layout_default(&output_layout, ZB_CHANNELS);
  SwrContext *swr = nullptr;
  result = swr_alloc_set_opts2(&swr, &output_layout, AV_SAMPLE_FMT_FLT,
                               ZB_SAMPLE_RATE, &codec_context->ch_layout,
                               codec_context->sample_fmt,
                               codec_context->sample_rate, 0, nullptr);
  if (result < 0 || swr == nullptr) {
    av_channel_layout_uninit(&output_layout);
    avcodec_free_context(&codec_context);
    close_format_input(&format, &custom_io);
    set_error(engine, "swr_alloc_set_opts2 failed: " + ffmpeg_error(result),
              ZB_ERR_DECODE);
    return;
  }

  result = swr_init(swr);
  if (result < 0) {
    swr_free(&swr);
    av_channel_layout_uninit(&output_layout);
    avcodec_free_context(&codec_context);
    close_format_input(&format, &custom_io);
    set_error(engine, "swr_init failed: " + ffmpeg_error(result),
              ZB_ERR_DECODE);
    return;
  }

  AVPacket *packet = av_packet_alloc();
  AVFrame *frame = av_frame_alloc();
  if (packet == nullptr || frame == nullptr) {
    cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                   &format, &custom_io);
    set_error(engine, "failed to allocate decode packet/frame", ZB_ERR_DECODE);
    return;
  }

  std::vector<float> converted;
  auto receive_frames = [&]() -> int32_t {
    while (!engine->decode_stop.load()) {
      const auto receive_result = avcodec_receive_frame(codec_context, frame);
      if (receive_result == AVERROR(EAGAIN) || receive_result == AVERROR_EOF) {
        return ZB_OK;
      }
      if (receive_result < 0) {
        return set_error(engine,
                         "avcodec_receive_frame failed: " +
                             ffmpeg_error(receive_result),
                         ZB_ERR_DECODE);
      }

      const auto out_samples = static_cast<int>(av_rescale_rnd(
          swr_get_delay(swr, codec_context->sample_rate) + frame->nb_samples,
          ZB_SAMPLE_RATE, codec_context->sample_rate, AV_ROUND_UP));
      const auto converted_size =
          static_cast<size_t>(out_samples * ZB_CHANNELS);
      if (converted.size() < converted_size) {
        converted.resize(converted_size);
      }
      uint8_t *output_data = reinterpret_cast<uint8_t *>(converted.data());
      const auto converted_samples =
          swr_convert(swr, &output_data, out_samples,
                      const_cast<const uint8_t **>(frame->extended_data),
                      frame->nb_samples);
      av_frame_unref(frame);
      if (converted_samples < 0) {
        return set_error(
            engine, "swr_convert failed: " + ffmpeg_error(converted_samples),
            ZB_ERR_DECODE);
      }

      const auto converted_frames = static_cast<int64_t>(converted_samples);
      const auto write_result =
          write_interleaved_to_ring(engine, converted.data(), converted_frames);
      if (write_result != ZB_OK) {
        return write_result;
      }
    }
    return ZB_OK;
  };

  while (!engine->decode_stop.load() && av_read_frame(format, packet) >= 0) {
    if (packet->stream_index == stream_index) {
      result = avcodec_send_packet(codec_context, packet);
      av_packet_unref(packet);
      if (result < 0) {
        cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                       &format, &custom_io);
        set_error(engine, "avcodec_send_packet failed: " + ffmpeg_error(result),
                  ZB_ERR_DECODE);
        return;
      }
      const auto receive_result = receive_frames();
      if (receive_result != ZB_OK) {
        cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                       &format, &custom_io);
        return;
      }
    } else {
      av_packet_unref(packet);
    }
  }

  if (!engine->decode_stop.load()) {
    avcodec_send_packet(codec_context, nullptr);
    const auto flush_result = receive_frames();
    if (flush_result != ZB_OK) {
      cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context,
                     &format, &custom_io);
      return;
    }
    const auto absolute_decoded = engine->decoded_frames.load();
    if (engine->duration_frames.load() <= 0 && absolute_decoded > 0) {
      engine->duration_frames.store(absolute_decoded);
      engine->duration_ms.store(ms_from_frames(absolute_decoded));
    }
    engine->buffered_ms.store(ms_from_frames(
        std::max<int64_t>(absolute_decoded, engine->duration_frames.load())));
    engine->decode_eof.store(true);
  }

  cleanup_ffmpeg(&packet, &frame, &swr, &output_layout, &codec_context, &format,
                 &custom_io);
}

int32_t wait_for_prebuffer(zb_engine *engine) {
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(ZB_OPEN_TIMEOUT_MS);
  while (std::chrono::steady_clock::now() < deadline) {
    if (engine->decode_failed.load() ||
        engine->state.load() == ZB_STATE_ERROR) {
      return ZB_ERR_DECODE;
    }
    const auto available = engine->pcm_rb_initialized
                               ? ma_pcm_rb_available_read(&engine->pcm_rb)
                               : 0;
    if (available >= ZB_PREBUFFER_FRAMES || engine->decode_eof.load()) {
      return ZB_OK;
    }
    std::this_thread::sleep_for(
        std::chrono::milliseconds(ZB_PREBUFFER_POLL_MS));
  }
  return set_error(engine, "timed out waiting for native decode prebuffer",
                   ZB_ERR_DECODE);
}

int32_t start_decode_worker(zb_engine *engine, const std::string &path,
                            int64_t start_ms) {
  engine->decode_stop.store(false);
  engine->decode_eof.store(false);
  engine->decode_failed.store(false);
  engine->decoded_frames.store(frames_from_ms(start_ms));
  engine->buffered_ms.store(start_ms);
  engine->cursor_frames.store(frames_from_ms(start_ms));
  engine->state.store(ZB_STATE_BUFFERING);
  try {
    engine->decode_thread =
        std::thread(decode_file_worker, engine, path, start_ms);
  } catch (const std::exception &error) {
    return set_error(engine,
                     std::string("failed to start native decode worker: ") +
                         error.what(),
                     ZB_ERR_DECODE);
  }
  return ZB_OK;
}

void apply_filter_to_interleaved(zb_engine *engine, float *frames,
                                 ma_uint32 frame_count) {
  if (engine == nullptr || frames == nullptr || frame_count == 0)
    return;

  const auto enabled = engine->filter_enabled.load() != 0;
  if (!enabled) {
    if (engine->filter_was_enabled) {
      engine->filter.reset();
      engine->filter_was_enabled = false;
      engine->filter_applied_type = ZB_FILTER_NONE;
      engine->filter_applied_cutoff_hz = 0.0f;
    }
    return;
  }

  const auto type = engine->filter_type.load();
  if (type != ZB_FILTER_LOW_PASS && type != ZB_FILTER_HIGH_PASS)
    return;
  const auto cutoff = std::clamp(engine->filter_cutoff_hz.load(), 20.0f,
                                 (ZB_SAMPLE_RATE / 2.0f) - 1.0f);
  if (!engine->filter_was_enabled || engine->filter_applied_type != type ||
      std::abs(engine->filter_applied_cutoff_hz - cutoff) > 0.5f) {
    if (!engine->filter_was_enabled || engine->filter_applied_type != type) {
      engine->filter.reset();
    }
    engine->filter.update(cutoff, ZB_SAMPLE_RATE,
                          static_cast<zb_engine_filter_type>(type));
    engine->filter_was_enabled = true;
    engine->filter_applied_type = type;
    engine->filter_applied_cutoff_hz = cutoff;
  }

  for (ma_uint32 frame = 0; frame < frame_count; ++frame) {
    auto *sample = frames + (frame * ZB_CHANNELS);
    engine->filter.process_stereo(sample[0], sample[1]);
  }
}

void audio_output_callback(ma_device *device, void *output, const void *,
                           ma_uint32 frame_count) {
  auto *engine = static_cast<zb_engine *>(device->pUserData);
  auto *frames = static_cast<float *>(output);
  if (engine == nullptr || frames == nullptr) {
    return;
  }

  const auto state = engine->state.load();
  if (state != ZB_STATE_PLAYING) {
    std::memset(frames, 0, sizeof(float) * frame_count * ZB_CHANNELS);
    return;
  }

  const auto gain = engine->volume.load();
  auto cursor = engine->cursor_frames.load();

  if (engine->using_ring_buffer && engine->pcm_rb_initialized) {
    ma_uint32 rendered = 0;
    while (rendered < frame_count) {
      auto readable = frame_count - rendered;
      void *read_buffer = nullptr;
      const auto acquire_result =
          ma_pcm_rb_acquire_read(&engine->pcm_rb, &readable, &read_buffer);
      if (acquire_result != MA_SUCCESS || readable == 0 ||
          read_buffer == nullptr) {
        break;
      }

      const auto *source = static_cast<const float *>(read_buffer);
      auto *destination = frames + (rendered * ZB_CHANNELS);
      const auto sample_count = readable * ZB_CHANNELS;
      if (gain == 1.0f) {
        std::memcpy(destination, source, sizeof(float) * sample_count);
      } else if (gain == 0.0f) {
        std::memset(destination, 0, sizeof(float) * sample_count);
      } else {
        for (ma_uint32 sample = 0; sample < sample_count; ++sample) {
          destination[sample] = source[sample] * gain;
        }
      }
      apply_filter_to_interleaved(engine, destination, readable);
      ma_pcm_rb_commit_read(&engine->pcm_rb, readable);
      rendered += readable;
    }

    if (rendered < frame_count) {
      std::memset(frames + (rendered * ZB_CHANNELS), 0,
                  sizeof(float) * (frame_count - rendered) * ZB_CHANNELS);
      if (!engine->decode_eof.load()) {
        engine->underrun_count.fetch_add(1);
      }
    }

    cursor += rendered;
    engine->cursor_frames.store(cursor);
    if (engine->decode_eof.load() &&
        ma_pcm_rb_available_read(&engine->pcm_rb) == 0) {
      engine->state.store(ZB_STATE_ENDED);
    }
    return;
  }

  const auto duration_frames = engine->duration_frames.load();
  const auto tone_gain = gain * 0.20f;
  for (ma_uint32 frame = 0; frame < frame_count; ++frame) {
    if (duration_frames > 0 && cursor >= duration_frames) {
      frames[(frame * ZB_CHANNELS)] = 0.0f;
      frames[(frame * ZB_CHANNELS) + 1] = 0.0f;
      engine->state.store(ZB_STATE_ENDED);
      continue;
    }

    const auto sample = static_cast<float>(std::sin(engine->phase) * tone_gain);
    frames[(frame * ZB_CHANNELS)] = sample;
    frames[(frame * ZB_CHANNELS) + 1] = sample;
    engine->phase += ZB_TAU * engine->frequency_hz / ZB_SAMPLE_RATE;
    if (engine->phase >= ZB_TAU) {
      engine->phase -= ZB_TAU;
    }
    cursor += 1;
  }
  apply_filter_to_interleaved(engine, frames, frame_count);
  engine->cursor_frames.store(cursor);
}

int32_t init_output_device(zb_engine *engine) {
  auto config = ma_device_config_init(ma_device_type_playback);
  config.playback.format = ma_format_f32;
  config.playback.channels = ZB_CHANNELS;
  config.sampleRate = ZB_SAMPLE_RATE;
  config.dataCallback = audio_output_callback;
  config.pUserData = engine;

  const auto result = ma_device_init(nullptr, &config, &engine->device);
  if (result != MA_SUCCESS) {
    return set_error(engine, "failed to initialize miniaudio playback device",
                     ZB_ERR_DEVICE);
  }
  engine->device_initialized = true;
  return ZB_OK;
}

int32_t open_decoded_source(zb_engine *engine, const char *source,
                            bool initialize_device, bool is_url) {
  if (engine == nullptr) {
    return ZB_ERR_INVALID_ENGINE;
  }
  if (source == nullptr || std::strlen(source) == 0) {
    return set_error(engine, "audio source must not be empty",
                     ZB_ERR_INVALID_ARGUMENT);
  }
  if (is_url) {
    avformat_network_init();
  }

  std::lock_guard<std::mutex> lock(engine->control_mutex);
  reset_stream_locked(engine);
  clear_error(engine);

  engine->state.store(ZB_STATE_OPENING);
  engine->source_uri = source;
  engine->phase = 0.0;
  engine->underrun_count.store(0);

  const auto ring_buffer_frames =
      is_url ? ZB_URL_RING_BUFFER_FRAMES : ZB_FILE_RING_BUFFER_FRAMES;
  auto result = init_ring_buffer(engine, ring_buffer_frames);
  if (result != ZB_OK)
    return result;
  if (initialize_device) {
    result = init_output_device(engine);
    if (result != ZB_OK)
      return result;
  }
  result = start_decode_worker(engine, engine->source_uri, 0);
  if (result != ZB_OK)
    return result;
  result = wait_for_prebuffer(engine);
  if (result != ZB_OK)
    return result;

  engine->state.store(ZB_STATE_READY);
  return ZB_OK;
}
} // namespace

zb_engine *zb_engine_create(void) { return new zb_engine(); }

void zb_engine_destroy(zb_engine *engine) {
  if (engine == nullptr) {
    return;
  }
  {
    std::lock_guard<std::mutex> lock(engine->control_mutex);
    engine->state.store(ZB_STATE_RELEASED);
    reset_stream_locked(engine);
  }
  delete engine;
}

int32_t zb_engine_open_generated_tone(zb_engine *engine, int64_t duration_ms) {
  if (engine == nullptr) {
    return ZB_ERR_INVALID_ENGINE;
  }
  if (duration_ms <= 0) {
    return set_error(engine, "generated tone duration must be positive",
                     ZB_ERR_INVALID_ARGUMENT);
  }

  std::lock_guard<std::mutex> lock(engine->control_mutex);
  reset_stream_locked(engine);
  clear_error(engine);

  engine->state.store(ZB_STATE_OPENING);
  engine->cursor_frames.store(0);
  engine->duration_frames.store(frames_from_ms(duration_ms));
  engine->duration_ms.store(duration_ms);
  engine->buffered_ms.store(duration_ms);
  engine->phase = 0.0;

  const auto result = init_output_device(engine);
  if (result != ZB_OK)
    return result;
  engine->state.store(ZB_STATE_READY);
  return ZB_OK;
}

int32_t zb_engine_open_file(zb_engine *engine, const char *path) {
  return open_decoded_source(engine, path, false, false);
}

int32_t zb_engine_open_url(zb_engine *engine, const char *url) {
  return open_decoded_source(engine, url, false, true);
}

int32_t zb_engine_prebuffer_file(zb_engine *engine, const char *path) {
  return open_decoded_source(engine, path, false, false);
}

int32_t zb_engine_prebuffer_url(zb_engine *engine, const char *url) {
  return open_decoded_source(engine, url, false, true);
}

int32_t zb_engine_play(zb_engine *engine) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }

  std::lock_guard<std::mutex> lock(engine->control_mutex);
  auto state = engine->state.load();
  if (state == ZB_STATE_PLAYING) {
    return ZB_OK;
  }
  if (state == ZB_STATE_ENDED && engine->using_ring_buffer &&
      !engine->source_uri.empty()) {
    if (engine->device_initialized) {
      ma_device_stop(&engine->device);
    }
    stop_decode_thread(engine);
    ma_pcm_rb_reset(&engine->pcm_rb);
    const auto restart_result =
        start_decode_worker(engine, engine->source_uri, 0);
    if (restart_result != ZB_OK)
      return restart_result;
    const auto prebuffer_result = wait_for_prebuffer(engine);
    if (prebuffer_result != ZB_OK)
      return prebuffer_result;
    state = ZB_STATE_READY;
  } else if (state == ZB_STATE_ENDED) {
    engine->cursor_frames.store(0);
    engine->phase = 0.0;
    state = ZB_STATE_READY;
  }

  if (state == ZB_STATE_READY || state == ZB_STATE_PAUSED) {
    if (!engine->device_initialized) {
      const auto init_result = init_output_device(engine);
      if (init_result != ZB_OK)
        return init_result;
    }
    engine->state.store(ZB_STATE_PLAYING);
    const auto result = ma_device_start(&engine->device);
    if (result != MA_SUCCESS) {
      return set_error(engine, "failed to start miniaudio playback device",
                       ZB_ERR_DEVICE);
    }
    return ZB_OK;
  }
  return set_error(engine, "play called before audio is ready",
                   ZB_ERR_INVALID_ARGUMENT);
}

int32_t zb_engine_pause(zb_engine *engine) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }
  if (engine->state.load() == ZB_STATE_PLAYING) {
    std::lock_guard<std::mutex> lock(engine->control_mutex);
    if (engine->device_initialized) {
      ma_device_stop(&engine->device);
    }
    engine->state.store(ZB_STATE_PAUSED);
  }
  return ZB_OK;
}

int32_t zb_engine_stop(zb_engine *engine) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }
  std::lock_guard<std::mutex> lock(engine->control_mutex);
  reset_stream_locked(engine);
  engine->phase = 0.0;
  engine->state.store(ZB_STATE_IDLE);
  return ZB_OK;
}

int32_t zb_engine_seek_ms(zb_engine *engine, int64_t position_ms) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }

  std::lock_guard<std::mutex> lock(engine->control_mutex);
  const auto duration = engine->duration_ms.load();
  const auto target_ms =
      std::clamp<int64_t>(position_ms, 0, std::max<int64_t>(duration, 0));
  const auto target_frames = frames_from_ms(target_ms);

  if (engine->using_ring_buffer && !engine->source_uri.empty()) {
    const auto was_playing = engine->state.load() == ZB_STATE_PLAYING;
    if (engine->device_initialized) {
      ma_device_stop(&engine->device);
    }
    stop_decode_thread(engine);
    ma_pcm_rb_reset(&engine->pcm_rb);
    engine->cursor_frames.store(target_frames);
    engine->decoded_frames.store(target_frames);
    engine->buffered_ms.store(target_ms);
    const auto restart_result =
        start_decode_worker(engine, engine->source_uri, target_ms);
    if (restart_result != ZB_OK)
      return restart_result;
    const auto prebuffer_result = wait_for_prebuffer(engine);
    if (prebuffer_result != ZB_OK)
      return prebuffer_result;
    if (was_playing) {
      engine->state.store(ZB_STATE_PLAYING);
      const auto play_result = ma_device_start(&engine->device);
      if (play_result != MA_SUCCESS) {
        return set_error(
            engine, "failed to restart miniaudio playback device after seek",
            ZB_ERR_DEVICE);
      }
    } else {
      engine->state.store(ZB_STATE_READY);
    }
    return ZB_OK;
  }

  const auto duration_frames = engine->duration_frames.load();
  const auto clamped = std::clamp<int64_t>(
      target_frames, 0, std::max<int64_t>(duration_frames, 0));
  engine->cursor_frames.store(clamped);
  engine->phase = 0.0;
  if (duration_frames > 0 && clamped >= duration_frames &&
      engine->state.load() == ZB_STATE_PLAYING) {
    engine->state.store(ZB_STATE_ENDED);
  }
  return ZB_OK;
}

int32_t zb_engine_set_volume(zb_engine *engine, float volume) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }
  engine->volume.store(std::clamp(volume, 0.0f, 1.0f));
  return ZB_OK;
}

int32_t zb_engine_set_http_headers(zb_engine *engine, const char *headers,
                                   const char *user_agent) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }
  std::lock_guard<std::mutex> lock(engine->control_mutex);
  engine->http_headers = headers == nullptr ? "" : headers;
  engine->http_user_agent = user_agent == nullptr ? "" : user_agent;
  return ZB_OK;
}

int32_t zb_engine_set_filter(zb_engine *engine, int32_t enabled,
                             zb_engine_filter_type type, float cutoff_hz) {
  if (is_released_or_null(engine)) {
    return ZB_ERR_INVALID_ENGINE;
  }
  if (enabled == 0) {
    engine->filter_enabled.store(0);
    engine->filter_type.store(ZB_FILTER_NONE);
    return ZB_OK;
  }
  if (type != ZB_FILTER_LOW_PASS && type != ZB_FILTER_HIGH_PASS) {
    return set_error(engine, "invalid native filter type",
                     ZB_ERR_INVALID_ARGUMENT);
  }
  engine->filter_type.store(type);
  engine->filter_cutoff_hz.store(
      std::clamp(cutoff_hz, 20.0f, (ZB_SAMPLE_RATE / 2.0f) - 1.0f));
  engine->filter_enabled.store(1);
  return ZB_OK;
}

zb_engine_state zb_engine_get_state(zb_engine *engine) {
  if (engine == nullptr) {
    return ZB_STATE_ERROR;
  }
  return engine->state.load();
}

int64_t zb_engine_get_position_ms(zb_engine *engine) {
  if (engine == nullptr) {
    return 0;
  }
  const auto position = ms_from_frames(engine->cursor_frames.load());
  if (engine->using_ring_buffer) {
    return position;
  }
  const auto duration = engine->duration_ms.load();
  return duration > 0 ? std::min(position, duration) : position;
}

int64_t zb_engine_get_duration_ms(zb_engine *engine) {
  return engine == nullptr ? 0 : engine->duration_ms.load();
}

int64_t zb_engine_get_buffered_ms(zb_engine *engine) {
  return engine == nullptr ? 0 : engine->buffered_ms.load();
}

int64_t zb_engine_get_decode_buffer_ms(zb_engine *engine) {
  if (engine == nullptr) {
    return 0;
  }
  const auto buffered_frames = frames_from_ms(engine->buffered_ms.load());
  const auto cursor_frames = engine->cursor_frames.load();
  return ms_from_frames(std::max<int64_t>(0, buffered_frames - cursor_frames));
}

int64_t zb_engine_get_ring_buffer_capacity_ms(zb_engine *engine) {
  return engine == nullptr
             ? 0
             : ms_from_frames(engine->ring_buffer_capacity_frames.load());
}

int64_t zb_engine_get_ffmpeg_probe_size_bytes(zb_engine *engine) {
  return engine == nullptr ? 0 : engine->ffmpeg_probe_size_bytes.load();
}

int64_t zb_engine_get_ffmpeg_max_analyze_duration_us(zb_engine *engine) {
  return engine == nullptr ? 0 : engine->ffmpeg_max_analyze_duration_us.load();
}

int64_t zb_engine_get_underrun_count(zb_engine *engine) {
  return engine == nullptr ? 0 : engine->underrun_count.load();
}

const char *zb_engine_get_last_error(zb_engine *engine) {
  if (engine == nullptr) {
    return "engine is null";
  }
  static thread_local std::string message;
  {
    std::lock_guard<std::mutex> lock(engine->error_mutex);
    message = engine->last_error;
  }
  return message.c_str();
}

int32_t zb_engine_analyze_silence_file(const char *path,
                                       int64_t *leading_silence_ms,
                                       int64_t *trailing_silence_ms) {
  return analyze_silence_file(path, leading_silence_ms, trailing_silence_ms);
}
