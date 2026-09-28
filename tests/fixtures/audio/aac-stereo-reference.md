# AAC stereo PCM reference

Source: existing `aac-stereo.aac` fixture (13 ADTS frames, 48 kHz, stereo).
Generated with FFmpeg 9.0.2 as a test oracle only:

```
ffmpeg -v error -i aac-stereo.aac -f f32le aac-stereo-reference.f32le
```

The file contains 26,624 interleaved little-endian float32 samples, including
initial decoder delay. Tests read the saved bytes; FFmpeg is not invoked by the
test or native decoding path. FVid uses a different PNS random sequence, so the
comparison is numerical rather than bit exact. Observed full-file RMS error was
0.000135962 and peak error 0.00264216; gates are 0.00015 and 0.003 respectively.
These bounds qualify this fixture, not every AAC tool or profile.
