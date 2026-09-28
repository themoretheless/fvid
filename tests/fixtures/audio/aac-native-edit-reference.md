# AAC MP4 edit-list fixture

Generated with FFmpeg 9.0.2, used only as fixture generator and independent
reference decoder. Runtime decoding/muxing uses FVid's owned implementation.

```
ffmpeg -v error -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=0.128' -c:a aac -b:a 64k -aac_pns 0 aac-native-edit.m4a
ffmpeg -v error -i aac-native-edit.m4a -f f32le -c:a pcm_f32le aac-native-edit-reference.f32le
```

The container presents 5645 mono samples at 44100 Hz after its priming edit.
The reference decoder emits a longer final AAC frame; compare its prefix only
through the MP4 edit endpoint. The FVid test asserts 5645 output samples and
peak PCM error below 1e-6, retaining full pre-roll decoder state.
