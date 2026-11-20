/* Research-only mono PCM capture and explicit refusal of MP4 transport.
   AAC math/parser and the generic ISO decoder frontend remain unchanged. */
#include <stdio.h>
#include <stdlib.h>
#include "audio.h"
#include "streamfile.h"

void OriginalAudioWriteData(AudioFile *, float **, long);

#ifndef FVID_RESEARCH_NO_CAPTURE
void AudioWriteData(AudioFile *file, float **data, long samples) {
    static FILE *output = NULL;
    if (!output) {
        const char *path = getenv("FVID_RESEARCH_CORE_PCM");
        if (!path || !(output = fopen(path, "wb"))) abort();
    }
    for (long i = 0; i < samples; ++i) {
        float value = data[0][i] / 32768.0f;
        if (fwrite(&value, sizeof(value), 1, output) != 1) abort();
    }
    if (fflush(output)) abort();
    OriginalAudioWriteData(file, data, samples);
}
#endif

int MP4initStream(HANDLE_STREAMFILE stream) {
    (void)stream;
    return -1;
}

void MP4showHelp(void) {
    fprintf(stderr, "MP4 unavailable in isolated LATM-only AAC research build\n");
}
