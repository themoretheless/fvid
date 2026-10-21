/* Optional independent oracle, never linked into FVid or ordinary tests.
 * Build against an explicitly supplied reference WavPack static library.
 * Usage: oracle input.wv output.f32le sample_frames
 */
#include "wavpack.h"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    char error[256];
    if (argc != 4 && argc != 5) return 2;
    WavpackContext *w = WavpackOpenFileInput(argv[1], error, argc == 5 ? OPEN_WVC : 0, 0);
    if (!w) { fprintf(stderr, "%s\n", error); return 3; }
    int channels = WavpackGetNumChannels(w), bits = WavpackGetBytesPerSample(w) * 8;
    unsigned count = (unsigned) strtoul(argv[3], NULL, 10);
    if (!count || channels < 1 || channels > 4096 || bits < 8 || bits > 32) return 4;
    int32_t *samples = calloc((size_t) count * channels, sizeof(*samples));
    if (!samples) return 5;
    unsigned got = WavpackUnpackSamples(w, samples, count);
    if (got != count || WavpackGetNumErrors(w)) {
        fprintf(stderr, "frames=%u/%u errors=%d\n", got, count, WavpackGetNumErrors(w));
        free(samples); WavpackCloseFile(w); return 6;
    }
    FILE *out = fopen(argv[2], "wb");
    if (!out) { free(samples); WavpackCloseFile(w); return 7; }
    double scale = (double) ((uint64_t) 1 << (bits - 1));
    int failed = 0;
    for (size_t i = 0; i < (size_t) got * channels; ++i) {
        float value = (float) (samples[i] / scale);
        const void *word = WavpackGetMode(w) & MODE_FLOAT ? (const void *) &samples[i] : (const void *) &value;
        if (fwrite(word, sizeof(value), 1, out) != 1) { failed = 1; break; }
    }
    if (fclose(out)) failed = 1;
    free(samples); WavpackCloseFile(w); return failed ? 8 : 0;
}
