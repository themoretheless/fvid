# High precision HEVC weighted prediction

Four synthetic HM 18.0 streams contain three 64x64 I/P/P pictures, at 8 and
10 bits, with high-precision offsets enabled or disabled. Weighted P prediction
is enabled in both counterparts. Saved YUV agrees with HM encoder reconstruction and HM decoding. FFmpeg is
also compared during generation; its 10-bit high-precision result differs.
The saved oracle for that case is HM, rather than FFmpeg.

Generate separately with `generate_hevc_context_samples.py --high-precision`
and the required HM encoder, decoder and configuration paths. Ordinary tests
read saved files only. Before the fix the enabled SPS failed with
`remaining HEVC SPS range-extension tools are not implemented`.

The acceptance test compares every decoded sample after reset and rewind, and
requires actual nonzero offsets in P-picture weight tables, including offsets
outside the 8-bit range in the enabled 10-bit stream. The native parser
uses depth-dependent luma/chroma offset bounds and chroma derivation; prediction
uses these offsets directly instead of applying the baseline bit-depth shift.
This implements H.265 equations 7-31 through 7-34 and 7-58.
