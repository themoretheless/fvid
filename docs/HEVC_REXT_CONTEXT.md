# HEVC range-extension significance contexts

The owned decoder implements `transform_skip_context_enabled_flag`. The
special significance contexts apply when the current block actually uses
transform skip or transquant bypass. Ordinary DCT/DST blocks use the original
context derivation even when the SPS flag is enabled. All bypass transform sizes
use the override; this is not limited to 4x4 blocks.

The normative rule is H.265 clause 9.3.4.2.5, equation 9-40:
[ITU-T H.265 V10 (2024)](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202407-S%21%21PDF-E&lang=e&type=items).
The combined SignificantCoefficient bank uses index 42 for luma and 43 for
chroma (the latter corresponds to the chroma-bank offset plus `sigCtx=16`).
Both contexts already had standard initialization; the fix connects actual
skip/bypass selection to the residual reader. Public compatibility entry points
retain their prior context-selection defaults.

## Synthetic acceptance fixtures

`tests/fixtures/playback-errors/hevc-rext-context-*` contains eight three-frame
64x64 synthetic streams: 8/10-bit 4:2:0, skip/bypass, and context enabled/disabled.
Generate separately from tests:

```sh
python3 scripts/generate_hevc_context_samples.py \
  --hm-encoder /path/to/TAppEncoder \
  --hm-decoder /path/to/TAppDecoder \
  --hm-config /path/to/HM/cfg/encoder_intra_main_rext.cfg
```

HM 18.0 directly encodes the desired CABAC contexts using
`SingleSignificanceMapContext`; it does not merely toggle SPS in an already
encoded stream. Other unsupported RExt tools are disabled. Transform sizes are
limited to 4x4 for these integration fixtures. Bypass fixtures force lossless
CU coding. The generator uses FFmpeg only for synthetic raw input, MP4 remuxing
and an additional independent decode. Encoder reconstruction, HM decoder and
FFmpeg decoder outputs must match exactly before the oracle is saved.
HM build provenance is recorded in [residual rotation](HEVC_REXT_ROTATION.md).
No private input, frames or codec parameter sets are used.

`tests/hevc_rext_smoothing.rs` checks every sample through the direct decoder
and software MP4 reader, including reset and rewind. It verifies profile, depth,
SPS context switch and PPS skip/bypass signalling. Enabled/disabled pairs must
have different VCL units after excluding parameter sets and other non-VCL NALs,
so metadata-only changes cannot pass. A scripted 8x8 residual unit case checks
special significance contexts across coded/uncoded coefficient groups while
retaining level-context carry and Rice reset behavior.

Before the fix, the enabled acceptance fixture reproduced specifically
`remaining HEVC SPS range-extension tools are not implemented`. The acceptance
test now compares decoded pixels rather than expecting a refusal. The obsolete
unsupported-flag expectation was removed; remaining unsupported tools still
have explicit refusal checks. Ordinary tests require neither FFmpeg, HM nor
network access. Other RExt tools and profiles/chroma formats remain incomplete.
