# AAC mono PCM reference

Source: existing `aac-mono-44k.aac`, AAC-LC mono at 44100 Hz.
Generated with FFmpeg 9.0.2 as a test oracle only:

```
ffmpeg -v error -i aac-mono-44k.aac -f f32le aac-mono-reference.f32le
```

Saved little-endian float32 PCM includes decoder delay. No FFmpeg execution
is required by the test or native decoder. Observed RMS error 0.0000315593,
peak error 0.000223995; gates 0.00004 and 0.0003. PNS sequences can differ, so
this is a bounded numerical comparison rather than bit-exact reproduction.
