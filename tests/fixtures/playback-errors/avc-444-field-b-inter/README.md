# Owned nonseparate 4:4:4 B fields

Generate: `python3 scripts/generate_avc_444_field_b_inter.py`.
Offline acceptance: `cargo test --offline --no-default-features --test avc_444_field_b_inter`.

864 six-packet I/P/B field sequences cross Y/C depths 8/8,10/12,14/8, both arrival
orders, CAVLC/CABAC init 0/1/2, two moving co-located references, L0/L1/bi/spatial/
temporal/skip and unweighted/explicit/implicit prediction. Source and selected
fields are owned; nested scalar six-tap interpolation supplies independent pixels.
Explicit luma/chroma denominators differ, and implicit weights use the selected
field's POC, not the containing complementary pair's minimum POC.

Native pair order is I/future-P/B. MP4 presentation must be [0,2,1]. Tests require
exact full-resolution woven planes and cropped normalized MP4 pixels, pending
field state, component depths, clocks, sample indexes, reset and rewind. A
positive temporal-motion witness excludes a zero-motion-average substitute.
No external decoder, FFmpeg, private media or network is used.

All B residuals are zero here. Nonzero B residual/tool combinations, mixed
frame/field transition sequences and adaptive-frame MBAFF inter require further
qualification; this is not an all-H.264 completion claim.
