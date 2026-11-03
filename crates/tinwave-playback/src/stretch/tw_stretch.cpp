// C ABI over Signalsmith Stretch (see tw_stretch.h). The library itself is the pinned header in
// vendor/signalsmith/; this file only adapts interleaved buffers to its channel-major indexing and
// keeps C++ exceptions from crossing the FFI boundary.
#include "tw_stretch.h"

#include <signalsmith-stretch/signalsmith-stretch.h>

#include <climits>
#include <cstring>
#include <exception>

namespace {

// `view[channel][frame]` over an interleaved buffer.
template <typename T>
struct Interleaved {
    T *data;
    size_t stride;

    struct Channel {
        T *base;
        size_t stride;
        T &operator[](int frame) const { return base[static_cast<size_t>(frame) * stride]; }
    };

    Channel operator[](int channel) const { return Channel{data + channel, stride}; }
};

constexpr int kMaxChannels = 8;
constexpr float kMinSampleRate = 8000.0f;
constexpr float kMaxSampleRate = 384000.0f;
// Constant seed: the random engine only matters for extreme stretch factors, and a fixed seed keeps
// the output reproducible for tests and for bug reports.
constexpr long kSeed = 0x5457;

using Stretch = signalsmith::stretch::SignalsmithStretch<float>;

bool frames_ok(size_t frames) { return frames <= static_cast<size_t>(INT_MAX); }

}  // namespace

struct tw_stretch {
    explicit tw_stretch(int channel_count) : stretch(kSeed), channels(channel_count) {}
    Stretch stretch;
    int channels;
};

extern "C" {

tw_stretch *tw_stretch_create(int channels, float sample_rate) {
    if (channels < 1 || channels > kMaxChannels) return nullptr;
    // Written so that NaN fails the check too.
    if (!(sample_rate >= kMinSampleRate && sample_rate <= kMaxSampleRate)) return nullptr;
    try {
        tw_stretch *handle = new tw_stretch(channels);
        try {
            handle->stretch.presetDefault(channels, sample_rate);
        } catch (...) {
            delete handle;
            return nullptr;
        }
        return handle;
    } catch (...) {
        return nullptr;
    }
}

void tw_stretch_destroy(tw_stretch *handle) { delete handle; }

void tw_stretch_reset(tw_stretch *handle) {
    try {
        handle->stretch.reset();
    } catch (...) {
        // reset() reuses the buffers it already owns, so nothing here can throw. A stretcher whose
        // reset failed half way would be unusable and the C ABI has no error channel for it.
        std::terminate();
    }
}

size_t tw_stretch_input_latency(const tw_stretch *handle) {
    return static_cast<size_t>(handle->stretch.inputLatency());
}

size_t tw_stretch_output_latency(const tw_stretch *handle) {
    return static_cast<size_t>(handle->stretch.outputLatency());
}

int tw_stretch_process(tw_stretch *handle, const float *in, size_t in_frames, float *out,
                       size_t out_frames) {
    if (!frames_ok(in_frames) || !frames_ok(out_frames)) return 1;
    if ((in_frames > 0 && in == nullptr) || (out_frames > 0 && out == nullptr)) return 1;
    try {
        Interleaved<const float> input{in, static_cast<size_t>(handle->channels)};
        Interleaved<float> output{out, static_cast<size_t>(handle->channels)};
        handle->stretch.process(input, static_cast<int>(in_frames), output,
                                static_cast<int>(out_frames));
        return 0;
    } catch (...) {
        return 2;
    }
}

int tw_stretch_flush(tw_stretch *handle, float *out, size_t out_frames) {
    if (!frames_ok(out_frames)) return 1;
    if (out_frames > 0 && out == nullptr) return 1;
    try {
        // flush() writes only the part of the tail that exists and folds the rest onto the
        // buffer's end, so the buffer must start zeroed.
        if (out_frames > 0) {
            std::memset(out, 0, out_frames * static_cast<size_t>(handle->channels) * sizeof(float));
        }
        Interleaved<float> output{out, static_cast<size_t>(handle->channels)};
        handle->stretch.flush(output, static_cast<int>(out_frames));
        return 0;
    } catch (...) {
        return 2;
    }
}

}  // extern "C"
