# Nonseparate 4:4:4 compressed intra loop filtering

Generate separately with `python3 scripts/generate_avc_444_deblock.py`.
Run `cargo test --offline --no-default-features --test avc_444_deblock`.

324 owned High 4:4:4 Predictive level 4.1 CAVLC videos contain four isolated
intra slices in a 32x32 frame. They cover 8/8, 10/12 and 14/8 Y/C precision,
independent Cb/Cr QP offsets, mixed QPY=26/30, alpha/beta offsets -6/0/+6,
disable_deblocking_filter_idc=0/1/2, external strong/short edges, internal I16
4x4 patterns and nonzero I8 residuals. All comparisons include complete coded
Y/Cb/Cr, cropped software MP4 output, native depth, PTS/duration/index, reset
and rewind. Expected samples use separate scalar Python equations and owned
8x8/filter helpers, never the decoder under test.

The author asserts nonzero witnesses against unfiltered output (71757 Y,
58075 Cb, 56557 Cr samples), wrong chroma QP thresholds (25435 samples), and
filtering odd 4x4 edges inside 8x8 transforms (15910 samples).

Baseline on 2026-10-21 rejected the first valid compressed sample specifically
with `AVC compressed 4:4:4 intra requires progressive CAVLC with deblocking
disabled`. These are playback acceptance tests, not passing refusal tests.
The first repaired version passed all 324 samples before the separate PCM
fast-path repair was applied.

H.264 (06/2019) 8.7 uses full-resolution chroma edge positions and sets
chromaStyleFilteringFlag=0 for ChromaArrayType=3. Component QPC and BitDepthC
remain independent of luma. Filtering runs after complete reconstruction.
CABAC compressed macroblocks and compressed field/MBAFF/inter are not covered.
All samples and parameters are synthetic; generation/tests need no FFmpeg,
network, private media or copied source codec parameters.

Final verification on 2026-10-21 passed all 324 samples after the PCM repair,
alongside the prior 60 compressed-intra cases and legacy 4:4:4 PCM/format tests.

Reauthoring verification on 2026-10-21 reproduced every generated artifact
byte for byte. Ordinary tests read these files without invoking the generator.
