# Owned Extended-profile partitioned B pictures

Generate separately with `python3 scripts/generate_avc_partitioned_b.py`.
There are 137 generated artifacts (182,366 bytes before this README). Ordinary
tests read stored artifacts and do not generate them or invoke external tools.
The generator also runs with PATH empty and commands/network denied. Extending
the shared writer left all 135 prior I/P/mixed-counter artifacts byte-identical.

Twenty-two 32x16 five-picture MP4s use 8-bit 4:2:0 Extended profile, two retained
references and no deblocking. All SPS/PPS, macroblocks and samples are synthetic;
no private pictures, codec parameter sets or external encoded streams are used.
Every PCM sample is nonzero under the Extended-profile constraint.

Decode order is IDR(POC0), future-I(POC6), B(POC2), B(POC4), successor-P(POC8).
The two references have independently authored coordinate samples differing by
four. Therefore L0, L1 and Bi produce different expected pixels (offsets 0/4/2).
Partition C adds a nonzero literal transform basis `[4,2,-2,-4]` in the selected
luma block. Partition B carries mixed PCM or intra4 residuals. Cases cover L0,
L1, Bi, temporal/spatial direct, B-skip and skip/coded transitions, PCM neighbours
in both orders, and reversed two-slice ASO. Both combined and split slices are
included. B partitions have nal_ref_idc=0; headers are separately checked for
actual B slice type, reference counts, frame number, POC and direct-mode flag.

Every packet has an independently serialized combined-stream control. Direct
native decoding compares each plane with both that control and scalar golden
samples over two reset passes. The following P must still use the future-I
reference, detecting accidental retention of non-reference B pictures. Forced
software MP4 decoding must reorder the five frames into presentation order with
exact PTS/duration, through EOF, rewind and sync seek. Literal chroma samples
additionally distinguish L0/L1/Bi without involving residual reconstruction.

These tests qualify this finite matrix, not every possible B tool combination.
Reference-coded B, explicit/implicit weighting, nonzero motion/reference-list
modification, SP/SI, FMO, field/MBAFF and missing-primary recovery need separate
partitioned-stream acceptance. Existing generic entropy readers passed these B
cases without a production decoder change; this is verification of the native
path, not a claim that all B behavior or all codecs are complete.
