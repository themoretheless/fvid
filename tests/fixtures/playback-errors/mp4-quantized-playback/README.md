# Quantized MP4 playback regression witnesses

Generate separately with `python3 scripts/generate_mp4_quantized_playback.py`.
Normal tests use checked-in bytes and require neither FFmpeg nor network access.
The generator emits 19 artifacts totaling 1,015,641 bytes (excluding this README).
All payloads are authored synthetic neutral video and audio; no private media or
codec parameter sets are copied.

`aac-playback-900.mp4` reproduces the original packet-duration alignment refusal.
`ssr-native-window.mp4` reproduces incomplete SSR PCM when the container adapter
confuses container durations with the native packet-clock selection contract.
Its float32 oracle is generated independently using scalar IPQF reconstruction.
`exact-*.mp4` preserve the authored encoded payloads before clock quantization.
The coarse variants live in the sibling `mp4-quantized-audio-clock` directory.

`tests/mp4_quantized_playback.rs` checks intended playback acceptance: exact PCM,
native rate/channel changes, presentation timestamps, delayed decoder output,
seek and rewind. It also runs the real scheduling worker with a capturing audio
backend. This is not physical output-device or arbitrary-input qualification.
