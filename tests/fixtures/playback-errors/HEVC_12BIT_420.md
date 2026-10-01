# HEVC 12-bit 4:2:0

Synthetic HM 18.0 streams contain three 64x64 pictures and saved planar
little-endian 12-bit reconstruction. Generation verifies HM encoder output,
HM decoder output agree. FFmpeg also agrees for the QP 24 fixtures; for
the QP 51 filtered pair it differs, so the authoritative saved oracle is HM. No reference tool is
needed during ordinary tests.

Generate with `scripts/generate_hevc_context_samples.py --depth 12`, supplying
HM encoder/decoder/configuration paths. `--filters` adds SAO and deblocking;
`--explicit` generates I/P/P with explicit RDPCM enabled/disabled.

`twelve_bit_420_matches_oracle` accepts skip and bypass cases, significance
contexts, filtered non-bypass pictures and inter RDPCM, comparing every sample
through decoder reset and MP4 playback rewind. Before the fix the first 12-bit
picture failed with `invalid HEVC picture parameters`.

The separate bypass-with-filters test is an explicit refusal, not acceptance:
that pre-existing tool combination still fails with `unsupported HEVC picture
tools`. Its paired synthetic streams/oracles are retained for the eventual fix.
`--mode skip --filters --qp 51` generates a separate maximum-QP pair.
`twelve_bit_high_qp_filters_match_oracle` reproduced `invalid HEVC edge
decision input` with the old 256-unit beta bound and accepts the corrected
1024-unit bound.
The camera bridge suite also verifies transport/seek for a filtered 12-bit
stream against direct software RGB output; this is not installed-device proof.

This covers 12-bit 4:2:0 without extended precision. Other chroma formats,
extended-precision processing, persistent Rice and high-throughput alignment
remain unsupported. PPS SAO offset scale is supported within `0..=bit_depth-10`. Generate the
scale-2 pair with `--depth 12 --mode skip --filters --sao-scale 2`.
`twelve_bit_scaled_sao_matches_oracle` previously reproduced a PPS failure
surfaced as `HEVC PPS has no valid SPS`. Acceptance requires nonzero scaled
SAO offsets and byte-exact HM output after reset and rewind. HM reconstruction,
HM decoding and FFmpeg reference decoding agree on these saved fixtures.

Offsets retain a 31-unit coded magnitude limit and are shifted by the PPS
scale before storage and application. Merged CTUs reuse the already scaled
values. The syntax unit test covers scale 0/1/2, signed maximum offsets,
clipping, merge without double scaling and out-of-range scale refusal.
