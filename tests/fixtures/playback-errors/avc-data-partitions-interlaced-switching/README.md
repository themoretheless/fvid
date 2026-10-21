# Owned partitioned interlaced switching pictures

Generate separately with
`python3 scripts/generate_avc_partitioned_interlaced_switching.py`.
The generator writes 2,051 artifacts (2,276,441 bytes before this README), with
PATH empty and external commands/network denied. Stored parameter sets and
samples are original synthetic data; all PCM seed samples are nonzero. Tests
never run the generator or invoke FFmpeg/network.

There are 184 four-frame 32x32 videos: 128 MBAFF primary/secondary SP cases and
56 PAFF primary/secondary SP, switching SI and ordinary-I-in-SI controls. Cases
cover QS0/26/51, zero/signed luma and chroma DC/AC residuals, all four adaptive
pair layouts, top/bottom/both skipped MBs, both field orders, single/split PAFF
slices and ASO with non-adjacent partitions. Whole skipped independent MBAFF
pairs infer frame mode; the oracle uses that actual inferred layout.

Each partitioned packet has an independently serialized combined-stream control.
Pixels come from owned integer-matrix luma/chroma switching oracles. Field AC
scan positions differ from frame positions, and complementary fields have
distinct samples. PAFF reference-list modification selects the previous frame's
same-parity field, even after a new opposite-parity field is available.

All samples and pair-completion behavior are checked through decoder reset.
Software MP4 checks four complete frames, exact 40ms clocks, EOF, rewind and sync
seek. Metadata checks 1,272 partitioned SP/SI headers, QP/QS, primary/secondary
flags, field parity/POC, actual B/C categories and packet byte bounds. An ordinary
SI intra control is invariant across QS and differs from switching SI.

These finite cases passed the existing decoder after the preceding PAFF fix;
no production code changed in this qualification step. They do not establish
MBAFF SI, arbitrary geometry, FMO/constrained/filter/weight/motion combinations,
missing-primary recovery or complete codec coverage.
