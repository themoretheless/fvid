# Nonseparate 4:4:4 shared-slice PCM/I16 fields

Author separately with `python3 scripts/generate_avc_444_fields_mixed.py`.
Run offline with `cargo test --offline --no-default-features --test avc_444_fields_mixed`.

108 owned level 4.1 High 4:4:4 Predictive videos combine PCM, compressed I16,
compressed I16 and PCM in each field's single slice. They cover CAVLC/CABAC,
both arrival orders, depths 8/8, 10/12 and 14/8, independent chroma offsets
0/+12/-12 and loop-filter IDC 0/1/2. CAVLC must retain independent nC=16 from
PCM neighbours. CABAC must restart its arithmetic engine after the complete
Y/Cb/Cr raw payload while retaining probability states and slice QPY=26;
PCM itself contributes filter QPY=0. Shared-slice IDC=2 edges still filter.

Expected samples use independent scalar reconstruction and field-aware loop
filter equations. Acceptance verifies complete woven components, physical
parity, crop/depth, software MP4 bytes, PTS/duration/index, reset and rewind.
All 108 cases passed on 2026-10-21; reauthoring matched all 541 artifacts.
No private media/parameters, FFmpeg, external decoder or network are required.
This matrix covers intra field transitions, not field inter or MBAFF frames.

Final 2026-10-21 verification also passed 215 AVC unit tests and 16 selected
AVC integration targets, including previous CAVLC/CABAC 444 frame/filter
matrices, ordinary monochrome/422 intra, component fields, separate-colour
fields and 8x8 scaling. Nonseparate MBAFF intra and frame/field inter now have dedicated matrices.
Adaptive MBAFF inter and unrepresented combinations remain open.
