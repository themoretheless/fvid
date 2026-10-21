# Owned nonseparate 4:4:4 PAFF frame pictures

Generate: `python3 scripts/generate_avc_444_paff_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_paff_inter`.

192 three-frame I/P/P videos use frame pictures under field-capable level-4.1
SPS geometry: frame_mbs_only=0, mb_adaptive_frame_field=0, field_pic=0, coded
32x32, crop [1,1,2,2]. They cross component depths 8/8,10/12,14/8, CAVLC/CABAC
init 0/1/2, integer/fractional motion, explicit component weights and skip/motion/
4x4 AC/8x8 DC. Four macroblocks exercise both top and left context contributions.
The scalar qpel oracle uses the full 32-row reference; 4x4 residuals use frame scan.

The baseline refused P frame 1 at the progressive-only SPS gate. Admission now
follows actual field/MBAFF picture syntax. Tests require an immediate frame
(no pending/complementary-field state), exact coded planes, cropped/normalized
MP4 bytes, clocks, reset and rewind. The final P-skip must sample the previously
reconstructed reference. No FFmpeg, system decoder, private media or network is
used. These are frame pictures, not mixed frame/field transition or MBAFF tests.
