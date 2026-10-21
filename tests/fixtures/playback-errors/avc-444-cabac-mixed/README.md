# CABAC 4:4:4 shared-slice PCM/I16 arithmetic restart

Author separately: `python3 scripts/generate_avc_444_cabac_mixed.py`.
Test offline: `cargo test --offline --no-default-features --test avc_444_cabac_mixed`.

27 owned progressive level 4.1 High 4:4:4 Predictive CABAC videos contain PCM,
compressed I16, compressed I16 and PCM macroblocks in a single 32x32 slice.
PCM reads full-resolution Y/Cb/Cr at independent depths, restarts only the
arithmetic engine, retains probability states and the slice's QPY=26, while
publishing PCM's filter QPY=0 and coded component contexts. Cases cover depths
8/8, 10/12 and 14/8, chroma offsets +/-12 and loop-filter IDC 0/1/2 at offsets
+12. IDC 2 still filters edges inside this shared slice.

Scalar sample/filter expectations match the independent CAVLC mixed-macroblock
oracle. All coded planes, cropped native-depth software MP4 output, PTS,
duration/index, reset and rewind are checked. All 27 cases passed on 2026-10-21.
No FFmpeg, network, private media or private codec parameters are needed.
These qualify progressive CABAC intra PCM transitions, not inter or field/MBAFF.

Final 2026-10-21 verification passed all 215 AVC unit tests and 14 selected
AVC integration targets, including ordinary monochrome/422 CABAC, inter
residuals, MBAFF loop filtering, component fields, separate-plane 8x8 scaling,
prior 444 CAVLC/PCM/filter matrices, and these CABAC cases. Reauthoring every
artifact in this directory independently reproduced identical bytes.
