# Covered partial arbitrary-tree grids

The previously built native player rejected the authored
`tree7-bands23-covered20` MP4 with the specific partial-band qualification
error. The same binary decoded its full-grid control:3 packets,3 decoded
windows,768 samples at24kHz. This isolates the new condition from core,
container and ordinary spatial-frame failures.

## Implemented boundary

A coded split need not span nominal parameter bands that the actual QMF domain
never addresses. Construction now checks every split against the canonical map
for the actual bank. In the32-bin domain these partial configurations qualify:

| Parameter bands | Covered partial cutoffs |
| --- | --- |
|12|11|
|15|14|
|23|20,21,22|

Every output-bin gain still uses explicitly coded CLD coefficients. CLD rows
retain their coded lengths; the owned decoder does not pad them. An invalid
later map that addresses an uncoded band fails before advancing gain history.
Without an explicit QMF domain, the public component constructor retains its
full-grid requirement. Ordinary full-band behavior is unchanged.

The decoder retains per-box cutoffs and accounts for that vector in its memory
footprint. Endpoint interpolation, fixed source gains, routes and synthesis
remain the previously qualified pipeline. This is not general active missing-
band processing: if even one actual bin is uncovered, the native constructor
still refuses that configuration.

## Independent source qualification

The explicit original-C reference harness now receives actual per-box cutoffs.
All84 existing full-band static/permutation outputs remained byte-identical
through that protocol change. For this work it compares20 covered partial
components (including nested trees and mixed cutoffs) and12 full controls.
Across196608 scalar outputs the maximum absolute error is
3.45915951698722e-10, below the2e-8 bound. Original C is neither linked into
production nor invoked by ordinary tests.

Two separately compiled copies of the unmodified original algorithm used
`-ftrivial-auto-var-init=zero` and `-ftrivial-auto-var-init=pattern`. The complete
covered/control outputs are byte-identical and finite. Six mixed-cutoff
uncovered probes instead differ in2880 scalars, with nonfinite values under
pattern initialization. Below-cutoff values remain identical. Source inspection
locates the affected stack gain rows: a partial parent does not initialize its
upper bands, while a wider descendant shifts/pops those rows at a wider cutoff.

The other six uniform-cutoff uncovered probes do not expose this dependence;
that observation alone is not their normative playback qualification. No
intended missing-band behavior is inferred from this audit. Compiler identity,
flags, original archive/algorithm/harness hashes and exact per-case differences
are recorded in `aac-ld-mps-arbitrary-partial-reference-2026-11-01.json`. Raw
uncovered outputs remain in the external reference cache, never playback gold.
The covered reference workspace initializes spare C storage only; the original
algorithm never reads that spare CLD data for covered outputs, and no such
padding is passed to the owned renderer.

## Media regressions

The authored corpus has80 covered playback programmes,64 full-grid controls
and68 explicit active-uncovered refusal reproducers. It exercises all eight
base trees, ordinary PCM and direct shared LD-SBR32 input, both decorrelation
modes, MP4 and Matroska. Separate header/frame
checks verify every coded cutoff and every parameter row length. The refusal
set includes64/128-bin cases and a nested mixed-cutoff video matching the
reference-audit condition. Those68 refusals are not acceptance of missing-band
playback.

Waveform comparisons use independently saved authored core PCM, owned base
spatial DSP and a separately written analytic additional split (absolute F32
tolerance1e-6), not a whole foreign decoder. Fixed LFE/surround gains are nonunity
in the applicable cases. The generator originally retained reference gains
while creating zero-gain-index headers; copying the intended gain fields into
the new headers corrected that fixture/oracle mismatch without loosening the
tolerance. Native tests check packet clocks, checkpoints and damaged-suffix
retry. Live readers compare full decode, nine seeks and rewind byte-for-byte.

Four focused integration targets completed20 tests with0 failures/ignored
tests. Exact commands and log hashes are recorded in the adjacent JSON. This
is not a full-current-tree gate. Authored generation reproduces448 files with
subprocess/socket creation forbidden. Ordinary tests need no FFmpeg, network,
C compiler, private source media or copied ISO media/parameter sets.

General active-uncovered partial processing, sparse positions, topology/grid
changes, nonzero EBQ with active arbitrary trees, active unequal-grid HRTF and
inherited-fragment prefix loss remain open. The overall codec-gap goal is not
complete.

The separate state unit passed (591 unrelated unit tests filtered out), bringing
the focused total to21 passed. The macOS probe completed12 MP4/Matroska runs,
representing plain PCM and shared LD-SBR, all five covered cutoff values and
3/7/9 source channels. Full playback, two seeks and rewind reached actual
default-device drain in all48 phases. This does not certify subjective quality
or physical multichannel wiring; raw output and executable SHA256 are in the
device JSON. The device build was awaited to terminal completion; no earlier
probe binary was substituted for this check.

Subsequent boundary update (2026-11-02): unchanged full-grid split paths now support resolution changes with dormant history retained. See [grid-change qualification](aac-ld-mps-arbitrary-grids-2026-11-02.md). Partial-grid conversion and topology changes remain open.
