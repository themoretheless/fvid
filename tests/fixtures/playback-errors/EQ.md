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

Owned admission currently covers constant numeric parameters on 8..=16-bit planar
YUV. Time/frame expressions, RGB and format conversion still
require implementation. The ordinary fixtures prove intended filter/export
acceptance, not completion of those remaining capabilities.

The existing two-frame synthetic `ffv1-gray-8.mkv` control is also an
acceptance fixture for owned equalization/export of monochrome FFV1.
`native_eq` checks backend, gray pixel format, every packet timestamp and
expected luma, plus neutral expanded chroma after owned decoding. Per-channel
chroma gamma must not turn a monochrome coded stream into a colour stream.
Regenerate the source with `generate_ffv1_gray_fixtures.py`.

`eq-precision-12.y4m` and `eq-precision-16.y4m` are three-frame 3x3
YUV444 controls generated with `scripts/generate_eq_high_depth_fixtures.py`.
They reproduce the former high-depth equalizer admission refusal. Library and
native CLI acceptance tests export Y4M/FFV1 with `gamma=2:saturation=0`,
checking distinct results for source values 1 and 2, quarter-scale gamma,
clipped endpoints, neutral chroma and half-second frame timestamps.
High-depth sample tables retain each source precision and are created once
per filter/depth, with fallible allocation; 8-bit lookup results stay unchanged.
Invalid high-depth samples are rejected before frame mutation.
