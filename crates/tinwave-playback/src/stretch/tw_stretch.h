/* C ABI over Signalsmith Stretch for tinwave-playback (T-0026, ADR 0006).
 *
 * Interleaved f32 audio only. Every function is safe to call from one thread at a time per handle.
 * `process` and `flush` never allocate and never throw. The Rust declarations live in ffi.rs and
 * must be kept in step with this file by hand. */
#ifndef TINWAVE_STRETCH_H
#define TINWAVE_STRETCH_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct tw_stretch tw_stretch;

/* Allocates and configures a stretcher with the default preset for `sample_rate`. Returns NULL for
 * an unsupported `channels` / `sample_rate` or when memory is exhausted. The random engine is
 * seeded with a constant, so equal input gives bit-identical output. */
tw_stretch *tw_stretch_create(int channels, float sample_rate);

/* Frees a handle from `tw_stretch_create`. NULL is allowed. */
void tw_stretch_destroy(tw_stretch *handle);

/* Back to the freshly configured state (history cleared). Does not allocate. */
void tw_stretch_reset(tw_stretch *handle);

/* Frames between a frame going in and the centre of its effect, and from the centre of the effect
 * to the output (the reported algorithmic latency). */
size_t tw_stretch_input_latency(const tw_stretch *handle);
size_t tw_stretch_output_latency(const tw_stretch *handle);

/* Consumes `in_frames` frames and fills `out_frames` frames; the ratio in/out is the playback rate.
 * Returns 0 on success, nonzero if the arguments are unusable (nothing was written). */
int tw_stretch_process(tw_stretch *handle, const float *in, size_t in_frames, float *out,
                       size_t out_frames);

/* Reads the tail of the stream with no further input. `out` is fully written (zero-filled past the
 * tail). Call `tw_stretch_reset` before feeding the stretcher a new stream. Returns 0 on success,
 * nonzero if the arguments are unusable. */
int tw_stretch_flush(tw_stretch *handle, float *out, size_t out_frames);

#ifdef __cplusplus
}
#endif

#endif
