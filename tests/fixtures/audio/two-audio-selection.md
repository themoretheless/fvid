# Selected AAC stream references

Source `two-audio.mp4` is the existing checked-in synthetic multi-track fixture.
On 2026-09-29, FFmpeg 9.0.2 was used only as a fixture/reference tool:

```sh
ffmpeg -i two-audio.mp4 -map 0:1 -map 0:2 -c copy two-audio.mka
ffmpeg -i two-audio.mp4 -map 0:1 -af atrim=start=0:end=0.04 -f f32le two-audio-stream1-reference.f32le
ffmpeg -i two-audio.mp4 -map 0:2 -af atrim=start=0:end=0.04 -f f32le two-audio-stream2-reference.f32le
ffmpeg -i two-audio.mka -map 0:0 -af atrim=start=0:end=0.04 -f f32le two-audio-mka0-reference.f32le
ffmpeg -i two-audio.mka -map 0:1 -af atrim=start=0:end=0.04 -f f32le two-audio-mka1-reference.f32le
```

Each PCM reference contains the first 40 ms of its selected stream. MP4 stream 0
is video; Matroska contains only the two copied AAC streams. Runtime/tests use the
owned parsers/decoder and saved PCM, without executing FFmpeg.

SHA-256:

```
d9a6cbb76ff27286d866eab7c321451e5d0970b4acc0f8035dc0cdc8c7a16124  two-audio.mp4
1b8d05eb4aaa96b44e6971dfd2ba8d053a476aefd2f3a03f6abd7877b4cb084d  two-audio.mka
fc7988db04eb12f7344c52f7767295799064d74f39fce4828dc77f94f22f132b  two-audio-stream1-reference.f32le
8f5e3f83c47b08d92a2f7934f0c2ff6fb5db246b5972a649bb767356a5493e6f  two-audio-stream2-reference.f32le
fc7988db04eb12f7344c52f7767295799064d74f39fce4828dc77f94f22f132b  two-audio-mka0-reference.f32le
8f5e3f83c47b08d92a2f7934f0c2ff6fb5db246b5972a649bb767356a5493e6f  two-audio-mka1-reference.f32le
```

The current owned decoder differs numerically from FFmpeg on these streams:
stream 1 peak 4.0732e-4 / RMS 6.8817e-5, stream 2 peak 2.8251e-4 / RMS 5.2198e-5.
Tests require peak <5e-4 and RMS <1e-4; they do not claim bit-exact equivalence
to FFmpeg. Own CLI/API and MP4/Matroska selected outputs are compared exactly.
