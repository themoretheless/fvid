# Full-grid ArbitraryTree resolution changes

An own MP4 with three valid configurations (23 -> 4 -> 23 parameter bands)
failed before the fix with the exact arbitrary-tree layout/grid-change refusal.
The base tree, output channels, split paths and core format were unchanged.

The original post-shaping algorithm stores `prevGainAT` over all 23 indexed
bands. Processing a smaller grid updates its active prefix without destroying
the rest. The owned component now uses the same fixed-size history, preserves
dormant values across full-grid changes, and writes only the current coded
prefix. New header parameters are still selected by their current extension
index; leaf history follows source/split paths, independently of output routing.
The new `inherit_history` API validates before copying, so errors are atomic.
Memory accounting already traverses each retained vector and includes the
expanded state. Reset restores every dormant and active band to unity.

This admits full-grid resolution changes with unchanged topology. A change
involving a partial split, or changed source/split paths, remains an explicit
refusal. Covered partial playback at a fixed grid remains supported. General
active-uncovered processing and topology conversion are not inferred here.

## Independent source comparison

The separate C harness changes the caller's parameter count, cutoffs and QMF map
between blocks, retaining the unchanged original `arbitraryTreeProcess` and its
23-band history. It covers all 42 ordered pairs of the seven parameter grids,
32/64/128-bin maps, simple and nested splits, and return to the original grid:
252 programmes and 1,185,408 scalar outputs. Its gold comes from the externally
cached ISO archive. Archive, algorithm, harness, input and output hashes are
pinned in the source receipt. Original source is neither vendored nor invoked
by ordinary tests. This is independent post-shaping component qualification,
not a foreign whole-audio-decoder comparison.

## Authored media and navigation

546 authored three-AU programmes cover every ordered grid pair on tree 7 in
all six previously qualified PCM/shared-LD-SBR domains, plus seven-grid cycles
on the other seven base trees. All have synthetic nonzero core/QMF sources,
explicit CLD and base parameters, and full splits. Every SSC/profile and spatial
frame is parsed completely before playback checks. MP4 video and Matroska must
produce identical nonzero PCM of the exact source duration. Both decorrelation
modes check packet clocks, checkpoints and damaged-packet retry. Live readers
check full decode, nine seek boundaries and rewind in both containers.

The explicit authored generators reproduce 1,097 files byte-for-byte with
subprocess/socket creation forbidden. The reference generation step is separate;
ordinary tests load saved component gold and media, without FFmpeg or network.
The focused test/device results and exact commands are in the adjacent JSON.
This milestone does not establish full codec parity. Active unequal HRTF grids,
EBQ with active arbitrary trees, sparse positions, topology changes and partial-
grid conversion remain open.

Validation completed: 21 unique focused tests passed, none failed or ignored.
The final component-only rerun also verifies that reset restores active and
dormant histories to a fresh component for every transition scenario. The
maximum scalar error against original C is 1.4688040332611063e-9, below 2e-8.
The C probe was explicitly compiled with `-DLD_MPS`; gold is byte-identical to
the initial generic-bank build. The rebuilt default-device probe completed
12 MP4/Matroska runs across all six input domains and 48 drains (full playback,
two seeks, rewind). This proves delivery to the device, not subjective listening
or physical multichannel wiring.
