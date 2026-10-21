# Nonseparate 4:4:4 CAVLC progressive P pictures

Generate separately: `python3 scripts/generate_avc_444_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_inter`.

90 owned three-frame videos cross Y/C depths (8/8,10/12,14/8), integer and signed
fractional motion, independent explicit component weights, P skip, motion-only,
4x4/8x8 residuals and an I16 block embedded after inter in the same P slice.
The 32x16 level-4.1 High 4:4:4 Predictive source is nonseparate, with crop [1;4].
Shared motion and common header/modes are read once. Each full-resolution plane
has its own CAVLC nC history, mapped QP, residuals and six-tap luma-style motion
compensation. Source PCM, predicted/residual frame and a following P-skip echo
are compared sample by sample; software MP4 output checks per-component packing,
crop, timestamps, reset and rewind. The echo proves the reconstructed reference
is published to the DPB, rather than only returning a temporary visible frame.

The baseline failed on frame 1 of `p-y8-c8-x0-y0-skip-w0` with
`unsupported inter-picture reconstruction tools`. This is actual decode
acceptance, not a refusal or a packet-index-only test. Scalar integer transforms
and six-tap interpolation are authored independently from the Rust decoder.
No FFmpeg, system codec, network or private source material is used.

Scope: this matrix is progressive CAVLC. CABAC now has separate acceptance in
`avc-444-cabac-inter` and its B/filter companions. Field/PAFF acceptance is covered separately; MBAFF inter remains open;
this matrix does not qualify every partition, scaling-list or slice combination.


Field/PAFF matrices: `avc-444-field-inter`, its B/filter companions and
`avc-444-paff-inter`. These extend admission without changing this matrix scope.
