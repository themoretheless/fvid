# Shared-slice 4:4:4 PCM/I16 filtering

Generate separately with `python3 scripts/generate_avc_444_mixed_deblock.py`.
Run `cargo test --offline --no-default-features --test avc_444_mixed_deblock`.

27 owned level 4.1 High 4:4:4 Predictive CAVLC videos combine PCM, compressed
I16, compressed I16, PCM macroblocks in one 32x32 frame slice. Each compressed
block borders PCM and requires nC=16 independently for Y/Cb/Cr. Predictions
come from the PCM edge averages; PCM returns filter QPY=0 but must not reset
the slice's QPY=26 for subsequent compressed blocks. Component offsets +/-12,
depths 8/8, 10/12 and 14/8, filter offsets +12 and IDC 0/1/2 are exercised.
IDC=2 must still filter shared-slice external edges. There are 4014 Y and
6352 samples in each chroma component changed by the scalar loop filter.

Acceptance compares every coded-plane sample and cropped software MP4 output,
depth, crop, PTS/duration/sample index, reset and rewind. Expected values use
separate owned scalar reconstruction/filter equations. No FFmpeg, network,
private sources or imported private codec parameters are needed.

All 27 cases passed on 2026-10-21. This qualifies progressive CAVLC intra;
compressed CABAC, inter and field/MBAFF remain outside this matrix.

Reauthoring verification on 2026-10-21 reproduced every generated artifact
byte for byte. Ordinary tests read these files without invoking the generator.
