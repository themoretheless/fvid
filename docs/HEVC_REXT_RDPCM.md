# HEVC implicit residual DPCM

The owned decoder implements `implicit_rdpcm_enabled_flag` for intra horizontal
(mode 10) and vertical (mode 26) blocks that actually use transform skip or
transquant bypass. Residual samples accumulate in raster order along the row or
column after skip scaling/bypass and any residual rotation. Ordinary transforms,
other intra modes and inter blocks retain their existing reconstruction.
Sign hiding is disabled for the implicit-RDPCM skip case. Angular prediction
boundary correction is disabled for transquant-bypass CUs when implicit RDPCM
is enabled, including blocks without coded residual samples. Reference-sample
smoothing and DC boundary filtering retain their separate rules.

Normative rules: H.265 residual-coding sign hiding, clauses 8.4.4.2.6 and 8.6.5,
and equations 8-322/8-323:
[ITU-T H.265 V10](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202407-S%21%21PDF-E&lang=e&type=items).
Integer additions are checked; malformed overflow aborts decoding rather than
publishing a partially reconstructed picture. Compatibility prediction and
residual entry points preserve their previous defaults.

## Synthetic acceptance

Generate eight three-frame 64x64 4:2:0 streams separately from tests:

```sh
python3 scripts/generate_hevc_context_samples.py --rdpcm \
  --hm-encoder /path/to/TAppEncoder \
  --hm-decoder /path/to/TAppDecoder \
  --hm-config /path/to/HM/cfg/encoder_intra_main_rext.cfg
```

The generator directly encodes implicit RDPCM using HM 18.0, with other
unsupported tools disabled, at 8/10 bits in skip/bypass and enabled/disabled
variants. HM encoder reconstruction and decoder output must match. In enabled
bypass variants they also must match the original synthetic raw input exactly.
HM provenance is in [residual rotation](HEVC_REXT_ROTATION.md). No private media
or parameter sets are used.

The local FFmpeg reference agrees with the non-bypass and RDPCM-disabled
variants. For enabled bypass it differed in 1,638 bytes at 8 bits and 1,705 bytes
at 10 bits from the lossless source/HM oracle. Those outputs are not saved as
acceptance oracles. Regeneration reports this comparison while retaining the
HM/source checks, so a future corrected FFmpeg can agree without changing the
acceptance criterion.

`tests/hevc_rext_smoothing.rs` checks every sample through direct decoding and
software MP4 playback, repeating after reset/rewind. It verifies the SPS flag,
profile, depth and PPS skip/bypass signalling, and requires different VCL
coding for enabled/disabled pairs. Scripted CABAC impulse tests independently
check horizontal and vertical accumulation for skip and bypass. Ordinary tests
require neither HM, FFmpeg nor network access.

Before the fix, the enabled fixture specifically reproduced
`remaining HEVC SPS range-extension tools are not implemented`. The obsolete
refusal expectation was removed and acceptance now requires decoded samples.
Explicit RDPCM, other RExt tools and other incomplete codec profiles remain
outside the implemented support.
