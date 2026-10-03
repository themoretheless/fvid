# Owned planar unsharp controls

Regenerate with `python3 scripts/generate_unsharp_fixtures.py`. All six
one-frame Y4M inputs are synthetic. The edge/ramp controls exercise 8/10/16-bit
precision, clamping, borders and large asymmetric kernels. Impulse controls
have independently calculated 3x3 binomial blur values; ordinary export tests
check those values after writing and decoding FFV1. No external codec or
runtime fixture generation is required by ordinary tests.

The optional `ffmpeg_unsharp_reference` benchmark compares fifteen parameter
and precision combinations with an independent decoder/filter, including
sharpening, negative amounts, zero amounts, and 23x3 kernels. Semantics are
referenced at https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_unsharp.c.

Owned production supports packed planar YUV at 8..16 bits with numeric luma
and chroma options. Alpha planes and their options are not supported by this
frame representation. The current separable implementation uses checked
frame-sized scratch; throughput optimization remains to be measured.
