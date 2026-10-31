# Same-format LD-MPS frame-length changes

336 authored twelve-core-packet programmes (MP4 and MKV),112 fixed-partition
controls in both containers,30 saved analytical bare-input controls and four
specific failing movies. No private media,audio or codec parameter sets.

SSC frame_slots is the spatial frame length,not the PCM output format.
All eight trees,480/512 AAC-LD and ELD cores,32/64/128-bin PCM ingress and
32/64-bin shared LD-SBR QMF ingress are exercised. New SSCs grow/shrink the
frame to8/15/16/30/32 slots while frequency and channel layout remain constant.
Headers arrive early,late or batched after all source packets. Initial spatial
frames are headerless and use ASC,including the first frame after reset.

The fixed controls retain the same constant absolute-coded spatial parameters,
unsmoothed endpoints and initial warm-up frame. Later frame repartitioning must
preserve the complete waveform exactly. This is a partition-invariance control,
not independent whole-codec PCM truth. The bare controls separately compare
saved analytical input PCM. The old production owners accept all fixed/bare
controls and refuse the variable-length movies at the exact old format guard.

The native AAC interface retains whole-core-packet frame grouping. Two movies
exercise its specific partial-core refusal;two others append a late trailing
byte after earlier changed-length frames in a batched submit. The latter tests
compare the exact Session queue/DSP state on failure and retry the repaired
suffix. They also verify native output/checkpoint behavior. A deliberately
incorrect full bank reset at each frame header is an observable negative control.

Tests cover MP4/MKV playback,seek/rewind,opaque packet windows,checkpoint replay,
reset from a longer frame to a headerless initial frame,and failed suffix retry.
Generate explicitly with
`python3 scripts/generate_aac_ld_mps_frame_length_fixtures.py`.
Ordinary tests consume frozen artifacts without generation,FFmpeg,network or
external reference programs.

This does not qualify changes to sample rate or channel layout,nonzero time
alignment,fractional-core native AAC frames,other quantization profiles or all
combinations with shaping/HRTF tools. No physical audio,realtime performance or
complete ISO23003-2 Annex B/profile claim.
