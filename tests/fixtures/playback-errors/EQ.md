# Owned constant equalization control

Regenerate with `python3 scripts/generate_eq_fixture.py`. The one-frame 16x16
YUV444 fixture contains all 256 possible sample values in each plane, with
independent permutations of chroma. Generation and ordinary tests need no
external codec or network access.

`native_eq` exercises public owned lossless export and native CLI export with
zero contrast/saturation, then decodes FFV1 and checks all 768 expected bytes.
It also checks native MP4 request/CLI decoding. The explicit optional
`ffmpeg_eq_reference` benchmark compares nine constant parameter combinations,
including per-channel gamma, negative contrast and extreme clipping.
Reference behavior is specified by
https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_eq.c and vf_eq.h.

Owned admission currently covers constant numeric parameters on 8-bit planar
YUV. Time/frame expressions, RGB and precision conversion still
require implementation. The ordinary fixtures prove intended filter/export
acceptance, not completion of those remaining capabilities.

The existing two-frame synthetic `ffv1-gray-8.mkv` control is also an
acceptance fixture for owned equalization/export of monochrome FFV1.
`native_eq` checks backend, gray pixel format, every packet timestamp and
expected luma, plus neutral expanded chroma after owned decoding. Per-channel
chroma gamma must not turn a monochrome coded stream into a colour stream.
Regenerate the source with `generate_ffv1_gray_fixtures.py`.
