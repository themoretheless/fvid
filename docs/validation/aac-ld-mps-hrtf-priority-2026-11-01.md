# Unused HRTF metadata with standard-inversion priority

The original TTT implementation gives standard inversion priority over HRTF
projection for modes 0, 1, 2 and 4. The assembler nevertheless required equal
spatial and HRTF grids before examining the active modes. The authored
`spatial23-hrtf15-priority` movie reproduced that specific false refusal before
the fix, after decoding its no-HRTF control successfully.

The assembler now retains HRTF structure/value validation but requires equal
grids only when at least one active band needs HRTF projection. The predicate
uses the actual per-band modes, including the high-region split. No DSP,
interpolation, phase cutoff, or general grid-conversion rule changes.

The priority corpus contains 49 grid-pair programmes, seven no-HRTF controls,
a repeated-header control, a programme changing unused HRTF metadata between
three resolutions, and an activation-refusal movie. Each has three authored
AUs and MP4 and Matroska video containers. Tests compare PCM against the
no-HRTF controls, both decorrelation modes and native clocks, playback rewind
and seek windows, and failure rollback followed by correct-packet retry.
A component test verifies that all four priority modes in trees 2/3/4 never
index unused four-band HRTF data even at spatial band 22.

The separate all-grid corpus has seven equal-grid acceptance programmes and
42 unequal-grid refusal reproductions. Those 42 refusals are not acceptance
of unequal-grid HRTF processing. Equal-grid integration compares saved
independent core PCM composed with owned spatial DSP; this is not a new
independent whole-spatial oracle. Four new equal-grid controls also have
recorded physical-device full/seek/rewind drains in the adjacent device JSON.

Both generators are explicit, authored, and separate from ordinary tests.
They reproduce 222 files byte-for-byte with subprocess and socket creation
forbidden. No private or ISO media or codec parameter sets were copied.

General active unequal-grid HRTF, unqualified EBQ combinations and arbitrary
inherited-fragment prefix loss remain open. This change does not complete
LD-MPS or the overall codec-gap goal. Full current-tree qualification is not
claimed by these focused tests.

The three focused test targets completed with 20 passed, 0 failed and 0 ignored.
The exact command, log SHA256 and scope are recorded in the adjacent JSON.
The existing 540-programme 3D suite retained its independent C/Decimal matrix
checks and zero-error composition over 24,330,240 PCM scalar values.
