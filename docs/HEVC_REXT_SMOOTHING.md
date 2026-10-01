# HEVC range-extension intra reference filtering

The owned software decoder implements `intra_smoothing_disabled_flag` from
SPS range-extension syntax. When set, it bypasses both weak and strong reference
sample filtering before planar/angular intra prediction. It does not disable
DC or angular prediction boundary correction. Existing Main/Main10 behavior and
public prediction/reconstruction entry points retain their original defaults.

The normative syntax and prediction rules are H.265 clauses 7.3.2.2.2 and
8.4.4.2.1:
[ITU-T H.265](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202407-I%21%21PDF-E&lang=e&type=items).

This, [residual rotation](HEVC_REXT_ROTATION.md) and
[significance contexts](HEVC_REXT_CONTEXT.md), and
[implicit RDPCM](HEVC_REXT_RDPCM.md) and
[explicit RDPCM](HEVC_REXT_EXPLICIT_RDPCM.md) are implemented RExt tools, not
complete format-range-extension profile support. Extended
precision, high-precision offsets, persistent Rice adaptation and CABAC bypass
alignment remain explicitly refused when signalled. Other extensions are also
refused. Existing chroma-format and picture-tool restrictions remain.

## Synthetic acceptance fixtures

Run `python3 scripts/generate_hevc_smoothing_samples.py` separately from tests.
The generator encodes three synthetic 64x64 `testsrc2` intra pictures for each
of 8-bit and 10-bit 4:2:0. It rewrites only VPS/SPS profile signalling to RExt
(no Main/Main10 compatibility claim), and adds SPS range-extension syntax with
all tool flags zero except the optional intra smoothing switch. It remuxes the
result into MP4 and saves independent decoded YUV oracles using FFmpeg.
The paired enabled/disabled oracles must differ, proving the fixture actually
exercises reference filtering. No private media or parameter sets are used.

`tests/hevc_rext_smoothing.rs` checks every YUV sample for all four fixtures
through the decoder and software MP4 playback paths, including decoder reset
and player rewind. It checks the signalled profile, depth and smoothing switch.
It also mutates each remaining RExt flag separately and checks the specific
unsupported-tool refusal. Tests require neither FFmpeg nor network access.

Before the fix, the acceptance fixture reproduced exactly
`HEVC SPS extensions are not implemented`. After the fix it compares decoded
samples against saved independent oracles; it is not a refusal-only test.
