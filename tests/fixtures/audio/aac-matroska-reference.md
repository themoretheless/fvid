# Matroska AAC PCM reference

Existing aac-stereo.mka decoded with FFmpeg 9.0.2 for independent validation:

```
ffmpeg -v error -i aac-stereo.mka -f f32le -c:a pcm_f32le aac-matroska-reference.f32le
```

48000 stereo sample frames, 48000 Hz, interleaved f32le. CodecDelay corresponds
to 1024 priming samples and final DiscardPadding to 128 samples. Nanosecond trim
metadata is rounded to nearest sample to recover muxer-rounded durations.
Native comparisons require identical length, RMS below 0.00015 and peak below
0.003 (independent PNS sequence); interval PCM must equal its full-decode slice.
FFmpeg is only a fixture/reference tool, never a runtime dependency of this path.
