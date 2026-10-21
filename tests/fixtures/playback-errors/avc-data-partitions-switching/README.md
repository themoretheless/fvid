# Owned partitioned SP/SI switching pictures

Regenerate separately with `python3 scripts/generate_avc_partitioned_switching.py`.
The generator writes 115 artifacts (127,334 bytes before this README), with no
external commands, network, copied codec parameter sets or private media. Tests
read stored artifacts without generating them. Every seed/PCM sample is nonzero
under the Extended-profile restriction.

Fourteen 32x16 four-picture MP4s use 8-bit 4:2:0 Extended profile, QPY26 and
QSY0/26/51. An ordinary PCM IDR seeds the reference state. The next pictures use
partitioned primary SP, secondary SP or SI. Each left macroblock exercises the
switching path; the right macroblock is ordinary PCM carried in B. Primary and
secondary SP include skipped and signed-coded inter paths, with C carrying all
signed luma and chroma residuals. SI includes signed/zero switching macroblocks
and an ordinary I_NxN control that must ignore switching quantization. Both
single-slice and reversed two-slice ASO variants are included.

Golden pixels are independently reconstructed with existing owned integer-matrix
luma and primary/secondary chroma oracle functions. They cover signed luma DC,
chroma DC and AC with different component/block signs, prediction from the last
reconstructed SP reference, current-picture SI DC prediction, the mapped chroma
QS39 at luma QS51, and ordinary PCM beside switching data. Every partitioned
packet has an independently serialized combined-stream control.

Native decoding checks every plane through reset. Forced-software MP4 checks
all four pictures, exact 40ms PTS/duration, EOF, rewind and sync seek. Header tests
check actual SP/SI type, QP/QS, primary/secondary flag, 63 slice headers, B/C
presence and exact packet byte caps. Golden controls separately prove that
ordinary SI-intra is invariant across QS, switching SI is different, and primary
SP signed residuals differ from skip. Thus a header-only or PCM-only path cannot
pass this fixture.

The existing generic partition readers passed this matrix without a production
code change. This is bounded acceptance, not full SP/SI qualification: weighting,
nonzero motion, constrained prediction, deblocking, FMO, field/MBAFF and recovery
from missing primary data still need partitioned-stream matrices.
