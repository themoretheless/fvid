# HEVC explicit inter residual DPCM

The owned decoder implements `explicit_rdpcm_enabled_flag`. In inter blocks
that actually use transform skip or transquant bypass, it reads
`explicit_rdpcm_flag` and, when true, `explicit_rdpcm_dir_flag` before the last
coefficient position. It accumulates the reconstructed residual horizontally
or vertically and disables sign hiding when explicit RDPCM is active. A false
block flag keeps the original residual; ordinary transforms and intra blocks
consume no explicit-RDPCM bins.

Normative rules: H.265 residual coding syntax (7.3.8.11), CABAC initialization
tables 9-32/9-33, and directional residual modification (8.6.5):
[ITU-T H.265 V10](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202407-S%21%21PDF-E&lang=e&type=items).
Both syntax banks use initialization value 139 for luma/chroma in P/B slices,
with no I-slice entries. They participate in the same CABAC snapshots as the
existing banks, including WPP state transfer. Compatibility residual entry
points retain explicit RDPCM disabled by default.

## Synthetic acceptance

Generate eight three-frame 64x64 4:2:0 streams separately from ordinary tests:

```sh
python3 scripts/generate_hevc_context_samples.py --explicit \
  --hm-encoder /path/to/TAppEncoder \
  --hm-decoder /path/to/TAppDecoder \
  --hm-config /path/to/HM/cfg/encoder_intra_main_rext.cfg
```

The generator derives a low-delay GOP of one I and two P pictures, using one
previous-picture reference. HM 18.0 directly encodes explicit RDPCM in skip and
bypass variants at 8/10 bits, with the SPS tool enabled/disabled. Unsupported
RExt tools are disabled. Encoder reconstruction, HM decoder and FFmpeg decoder
must agree before saving each YUV oracle. HM provenance is recorded in
[residual rotation](HEVC_REXT_ROTATION.md). No private media or parameter sets
are copied, and generation is separate from test execution.

`tests/hevc_rext_smoothing.rs` verifies actual I/P/P slice headers (preventing
intra-only fixtures from passing), SPS signalling, profile/depth and PPS
skip/bypass settings. It compares every YUV sample through direct decoding and
software playback, repeating after reset and rewind. Enabled/disabled pairs
must have different VCL coding. Scripted CABAC unit cases verify flag/direction
ordering, both directions and the inactive case, luma/Cb/Cr context selection,
and skip/bypass reconstruction.

Before the fix, the enabled fixture reproduced specifically
`remaining HEVC SPS range-extension tools are not implemented`. The obsolete
refusal expectation was removed and replaced by decoded-frame acceptance.
Ordinary tests use saved fixtures and require neither HM, FFmpeg nor network.
Other unfinished tools/profiles and broader WPP/B-slice RExt fixture coverage
remain; this does not claim complete HEVC support.
