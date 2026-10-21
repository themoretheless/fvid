# Full 4:4:4 I frames under field-capable PAFF SPS

Author separately with `python3 scripts/generate_avc_444_paff_frames.py`.
Run offline with `cargo test --offline --no-default-features --test avc_444_paff_frames`.

54 owned 32x32 level 4.1 High 4:4:4 Predictive pictures use an SPS with
frame_mbs_only_flag=0, mb_adaptive_frame_field_flag=0 and height_map_units=1,
but code a complete frame (field_pic_flag=0). They cover CAVLC/CABAC I16,
Y/C depths 8/8, 10/12, 14/8, chroma QP offsets 0/+12/-12 and filtering IDC
0/1/2. Their full coded height is 32, unlike a compact 16-row field using
that SPS. Cropped output is 30x28 with crop [1,1,2,2].

Acceptance requires immediate native frame output, no pending field and no
field-pair output marker, exact coded planes and software MP4 bytes, PTS 0,
duration 40 ms, sample index 0, reset and rewind. All 54 cases passed on
2026-10-21; reauthoring reproduced all 217 artifacts byte for byte.

All samples, parameters and scalar oracles are owned. Generation and ordinary
tests need no FFmpeg, external decoder, network or private sources. This is
full intra-frame PAFF geometry coverage, not inter or MBAFF qualification.

Final 2026-10-21 verification also passed 215 AVC unit tests and 16 selected
AVC integration targets, including previous CAVLC/CABAC 444 frame/filter
matrices, ordinary monochrome/422 intra, component fields, separate-colour
fields and 8x8 scaling. Nonseparate MBAFF intra and frame/field inter now have dedicated matrices.
Adaptive MBAFF inter and unrepresented combinations remain open.
