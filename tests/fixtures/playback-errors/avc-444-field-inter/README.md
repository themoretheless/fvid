# Owned nonseparate 4:4:4 P fields

Generate: `python3 scripts/generate_avc_444_field_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_field_inter`.

768 four-packet videos cross Y/C depths 8/8,10/12,14/8, CAVLC and CABAC init
0/1/2, both complementary arrival orders, matching/opposite reference parity,
integer/fractional motion, independent component weights and skip/motion/4x4 AC/
8x8 AC. Owned level-4.1 High 4:4:4 Predictive SPS is field-capable, nonseparate,
not MBAFF: coded full geometry 32x32, compact fields 32x16, crop [1,1,2,2].

The first baseline failed on P field 2 of the skip stream with `invalid AVC P
field configuration`. The full-resolution shared entropy path now reads one
common header/motion description and three independent component residual grids.
Compact selected-field pixels remain borrowed. Field reference identity includes
parity; motion and POCs remain in selected-field coordinates. All components use
six-tap luma-style compensation, without the 4:2:0 chroma parity correction.

The scalar 4x4/8x8 AC oracles require the field scan to differ from the frame
scan; `witnesses.json` records changed-sample witnesses. Native acceptance checks
pending/completed complementary state, exact woven planes, precision, crop and
CABAC init declarations. Software MP4 must match exact cropped/normalized pixels,
clocks and sample indexes through reset and rewind. No FFmpeg, system decoder,
network or private source material is used by generation or ordinary tests.

This matrix has filter mode 1; component filtering is qualified separately.
Mixed slice/tool combinations beyond the matrix and adaptive-frame MBAFF inter
remain separate work. The matrix does not claim complete H.264 support.
