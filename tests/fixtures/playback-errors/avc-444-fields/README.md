# Nonseparate 4:4:4 CAVLC/CABAC intra fields

Author separately with `python3 scripts/generate_avc_444_fields.py`.
Run offline with `cargo test --offline --no-default-features --test avc_444_fields`.

540 owned High 4:4:4 Predictive level 4.1 videos contain complementary compact
32x32 fields woven into a 32x64 frame. The 432 compressed and 108 PCM cases
cover CAVLC/CABAC, both physical-field arrival orders, Y/C precision 8/8,
10/12 and 14/8, independent chroma QP offsets 0/+12/-12 and filter IDC 0/1/2
at alpha/beta offsets +12. Every field contains four independently coded slices.

Compressed patterns include I16 DC, different nonzero field-scan positions in
Y/Cb/Cr I16 DC, I4 field-scan AC and I8 field-scan AC. I8 scan position 3 maps
to raster position 1 in a field (rather than position 16 in a frame). The
scalar expectations model intra prediction and integer reconstruction before
field-aware loop filtering. Treating horizontal field edges as frame edges
changes 43444 expected samples across the matrix. Top and bottom field samples
must differ, so a parity swap cannot pass.

The baseline rejected compressed fields specifically with `AVC 444 compressed
I fields are not connected`. A separate PCM case returned unfiltered Cr sample
120 where 121 was required. Both are enabled sample-acceptance regressions.

Acceptance verifies the pending-field transition, complete Y/Cb/Cr samples,
independent depths, woven physical parity, crop [1,1,2,2], visible size 30x60,
software MP4 packed bytes, sample index 0, PTS 0, paired duration 40 ms, reset
and rewind. All 540 cases passed on 2026-10-21. Independent reauthoring matched
all 2701 generated artifacts byte for byte.

No private source media or codec parameters, FFmpeg, external decoder or network
are used by authoring or ordinary tests. This qualifies nonseparate intra
fields. Separate acceptance now covers field/PAFF inter and MBAFF intra;
see the dedicated matrices. Adaptive MBAFF inter remains separate work.

Final 2026-10-21 verification also passed 215 AVC unit tests and 16 selected
AVC integration targets, including previous CAVLC/CABAC 444 frame/filter
matrices, ordinary monochrome/422 intra, component fields, separate-colour
fields and 8x8 scaling. Nonseparate MBAFF intra and frame/field inter now have dedicated matrices.
Adaptive MBAFF inter and unrepresented combinations remain open.
