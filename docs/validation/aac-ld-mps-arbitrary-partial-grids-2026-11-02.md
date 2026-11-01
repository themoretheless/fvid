# Covered uniform-cutoff ArbitraryTree grid changes

A new own MP4 changing 12/cutoff11 -> 23/full -> 12/cutoff11 failed before the
fix specifically at the layout/grid-change refusal. Every actual 32-bin QMF
coefficient has coded CLD at every step. The refusal was therefore broader than
the undefined active-uncovered behavior documented in the earlier source audit.

The history transfer now admits resolution changes when the additional splits
have a uniform cutoff within each configuration. All active coefficients must
still be covered by the canonical QMF map, as checked by construction/process.
The indexed 23-band history updates only the coded prefix and preserves dormant
values. Both partial/full and partial/partial transitions can return to the
original grid. Output/source paths and channel count remain unchanged.

A mixed-cutoff grid change remains refused: the prior original-C audit exposed
undefined dormant rows in that configuration. This milestone does not infer
normative missing-band values or apply such undefined rows to newly active bins.
Same-grid covered mixed-cutoff playback remains the previously qualified case.
Topology changes, sparse positions, active-uncovered bands, EBQ with active
arbitrary trees and active unequal HRTF grids remain open.

The authored corpus contains 218 accepted three-AU programmes across all eight
base trees and plain/shared-LD-SBR32 domains. Tree7 covers all74 ordered state
pairs with distinct nominal grids and at least one covered partial state; other
trees cover each of the five partial cutoffs with return. Seven full grids and
five covered-partial states participate. MP4 video and Matroska must produce
identical nonzero PCM of the exact source duration. Both decorrelation modes
check packet clocks, checkpoints and damaged-packet retry; readers check nine
seek boundaries and rewind.

Two additional mixed-cutoff videos are fully parsed and initially playable.
They reproduce the precise refusal only upon the second header's grid change.
The test verifies retained memory, valid retry and checkpoint equivalence after
that failure. These refusals are not acceptance of mixed-cutoff conversion.

Independent original-C post-shaping gold covers 148 simple/nested programmes,
all74 state pairs with return, and298368 scalars. The original algorithm is
unchanged; the caller supplies per-block grid, uniform cutoffs and canonical map.
Only the C harness's fixed input storage is padded above coded CLD lengths;
the original algorithm never reads that padding. Owned tests supply actual
short rows and retain exact coded lengths. This is component qualification,
not a foreign whole-decoder waveform comparison. Original source is external,
never linked into production or invoked by ordinary tests.

The two authored generators reproduce445 files byte-for-byte with subprocess
and socket creation forbidden. Reference compilation is an explicit separate
operation. Ordinary tests require no FFmpeg, network or compiler. Final focused
and device results are recorded in adjacent JSON receipts. Full codec parity
remains unproven.

Validation completed: 20 unique focused tests passed, none failed or ignored.
The maximum original-C scalar error is 1.4688040332611063e-9, below2e-8.
Reset also restores active/dormant state to a fresh component for every source
scenario. The rebuilt default-device probe completed12 MP4/Matroska runs and
48 drains (full playback, two seeks, rewind), spanning both input domains and
all five partial cutoff values. This is device delivery evidence, not subjective
listening or physical multichannel wiring certification.

Subsequent boundary update (2026-11-02): mixed-cutoff resolution changes now transfer exactly when every newly addressed band has defined history. See [defined-history qualification](aac-ld-mps-arbitrary-defined-2026-11-02.md). The blanket mixed-cutoff refusal above describes the historical snapshot.
