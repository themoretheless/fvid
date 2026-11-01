# Nonzero EBQ with an active ArbitraryTree: reference-source audit

This milestone does not add nonzero-EBQ ArbitraryTree playback. It turns an
unqualified boundary into independently reproduced source evidence and precise
container-level regressions. The existing owned refusal remains in production.

The unchanged external ISO `sac_bitdec.c` calls `mapIndexData` for additional
OTT CLD with `self->quantMode` and three NULL energy-array arguments.
`mapIndexData` requires those arrays whenever CLD quantization is nonzero.
A separate research-only C harness invokes this original caller with fully
zero-initialized lossless state (legal default data mode), one active 23-band
split and one parameter set. With assertions enabled, modes 1 and 2 both abort
specifically at `ottVsTotDbIn`; mode 0 returns a finite default CLD. No original
algorithm bytes are patched. The external source is not vendored or linked
into FVid. This demonstrates a source API defect, not the normative energy
propagation that ought to replace it; it is not a playback oracle.

The authored corpus contains 18 quant-zero playback controls and 36 nonzero
refusal programmes: trees 0, 1 and 2 across six PCM/shared-LD-SBR domains,
quantization modes 0, 1 and 2, three access units, MP4 video plus Matroska.
Header/profile/frame syntax is fully parsed before asserting the exact energy-
quantization refusal. Container refusals emit no PCM. They are explicitly not
acceptance of intended nonzero-EBQ playback. The control programmes compare
continuous waveform composition, both decorrelation modes, source clocks,
checkpoint restore, damaged-packet retry, seeks and rewind.

Four automated tests pass with no failures or ignored tests. An initial test
used the default output rate for downscaled programmes, causing an unrelated
core-geometry error; selecting each authored programme's actual output rate
corrected the test setup. It did not alter the runtime guard or tolerances.
The explicit generator reproduces all 128 authored files byte-for-byte while
subprocess and socket creation are forbidden. Ordinary tests do not run the
reference compiler, generator, FFmpeg or network.

A renewed normative search found indexed 2010 and 2018 full-PDF candidates,
but both returned HTTP 404 rather than PDF. Preview documents do not establish
the missing Annex B semantics. These failed candidates are not source authority
for implementation. The original ISO reference archive and algorithm hashes,
compiler identity, flags and exact abort messages are recorded in the source
receipt. No guessed zero-energy or inherited-parent workaround is enabled.
Topology/grid changes, active-uncovered partial splits, sparse output positions,
active unequal HRTF grids and nonzero EBQ with active trees remain open.
