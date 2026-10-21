# Owned partitioned PAFF fields and MBAFF pictures

Generate separately using `python3 scripts/generate_avc_partitioned_interlaced.py`.
The generator writes 2,227 artifacts (3,960,038 bytes before this README), runs
with PATH empty and external commands/network denied, and copies no private
media or parameter sets. All SPS/PPS, nonzero PCM samples and signed residuals
are synthetic. Ordinary tests read artifacts without generating them.

The matrix has 240 32x32 three-frame videos: 192 MBAFF and 48 complementary PAFF
field sequences. Cases cover both uniform seed layouts, all four pair layouts,
layout changes between references, top/bottom-first field order, coded P,
PCM-left/inter-right, inter-left/PCM-right, partitioned I/PCM, one/two slices,
ASO with interleaved A/B/C and optional explicit two-group FMO. The PAFF header
explicitly chooses the previous frame's same-parity field so that a newly decoded
opposite-parity field cannot become an unintended reference.

Golden pixels use physical coordinate samples and a literal signed transform
basis. Frame-MBs apply `[4,2,-2,-4]` horizontally; field-MBs apply it vertically
with a two-row physical stride. Groups/pairs have opposite signs. Each frame is
updated from the preceding reference, resetting PCM regions while preserving
existing residual history across layout changes. This detects wrong scan order,
field parity, pairing, reference selection and B/C ownership. Every packet has
an independently serialized combined-stream control.

Native decoding compares all samples and exact pair-completion behavior through
reset. Software MP4 verifies all three complete frames, exact 40ms PTS/duration,
EOF, rewind and sync seek. A direct PCM-field API test verifies 32 partitioned
fields, independently extracting even/odd rows. The header test checks 960
partitioned headers: field flags, POC/frame_num, reference-list modification,
pair-unit IDs, B/C presence/offsets and exact packet byte caps.

The original PAFF coded path read coefficients from A and specifically failed
with `AVC P field skip exceeds picture`; the direct PCM path read samples from A
and failed with `truncated or oversized bit field`. Both baselines were captured
before enabling the fixes. The fixes route C to inter coefficients and B to PCM,
finish both independent streams, and reject surplus B/C on the pure-skip copy
path. Pure-skip acceptance proves that copy behavior remains valid. Five owned
negative packets and short MP4s cover missing B/C and surplus B/C, require the
intended error, and test reset/rewind recovery separately from acceptance.

This finite matrix does not establish partitioned B/SP/SI interlaced combinations,
nonzero motion, weighting, deblocking, arbitrary dimensions/groups, or recovery
from missing primary data. Full codec completion remains unproven.
