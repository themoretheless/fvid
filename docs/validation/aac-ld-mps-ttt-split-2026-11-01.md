# Predictive TTT split-region validation

The authored predictive-default corpus now adds twelve programmes with separate
low/high regions: trees 525/7271/7272, grid9, split at band4, modes0/1 and1/0,
and both initial-default and coded/default/hold histories. Each has an explicitly
coded unity control, in MP4 and Matroska. The total is48 programmes,48 controls,
192 movies. These remain three nonzero authored ELD core AUs each; no private
or conformance-stream media/parameters are copied.

Playback acceptance compares complete default/hold PCM byte-for-byte with its
coded control. Both container readers exercise initial playback, rewind and
five seek positions; repeated MP4 intervals and native checkpoint replay remain.
Ordinary tests use only checked-in artifacts, without FFmpeg or network access.

## Independent reference boundary

The unmodified pinned ISO spatial decoder processes all96 programme/mode pairs
(48 programmes, Real and Complex decorrelators). Default/hold and coded unity
are byte-identical in every original-reference pair. The full original PCM is
preserved in `reference.f32le`, including the discrepancy described below.
The original AAC core is not being qualified by this spatial comparison.

For92 outputs, native PCM and unmodified ISO PCM agree within maximum absolute
error1.1496013e-9, at zero lag, without gain/delay/channel fitting. The limit
remains1e-7 full scale.

Four outputs (tree525, lower mode0/upper mode1, two histories, both decorrelators)
differ from the unmodified ISO renderer. The original `sac_bitdec.c` TREE_525
configuration loop assigns `numDecorSignals=3` for lower mode0, then overwrites
it with2 for upper mode1. Lower-region `useTTTdecorr` remains1. Consequently
the global V/W geometry omits a branch still requested by that region.
The reverse mode order ends with3 and matches native PCM.

A separately built diagnostic variant changes only that TREE_525 assignment
from2 to `(lower_mode == 0 ? 3 : 2)`. Original sources are restored immediately
after compiling that executable. The source before/after hashes and exact
replacement are recorded in the reference JSON. Its four complete outputs
match native within the same1e-7 limit (observed combined maximum1.1496013e-9).
Default/coded controls remain byte-identical in this diagnostic too.

`count-control.f32le` is explicitly a modified-source diagnostic, not an
unmodified ISO or normative conformance oracle. Tests retain the original
mismatch as a specific reproduction and compare the diagnostic separately.
No production DSP is changed to imitate the reference count overwrite. A full
normative qualification of this combination remains open pending independent
specification evidence; this report does not declare all LD-MPS gaps closed.

Six actual macOS `audio_probe --play-device` runs cover both split mode orders
on all three trees. Each completes full playback,25% seek,60% seek and rewind
through device EOF drain:24 phases. These32ms movies verify scheduling/drain,
not subjective audio quality or sustained performance. Logs are retained in
`aac-ld-mps-ttt-split-device-2026-11-01.json`.

## Reference availability

The official source archive remains:
https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip

The current search did not retrieve full ISO/IEC23003-2 AnnexB/TableA.21.
The iTeh previews stop before those sections, and the indexed ieclist full-PDF
URL returns404. Catalog pages and indexed snippets are not used as algorithmic
proof. Unequal HRTF grids, unqualified EBQ combinations and replaced-fragment
source association remain open; this validation does not implement them.

Focused verification passes13 tests across `aac_ld_mps_ttt_default`,
`aac_ld_mps_frame`, `aac_ld_mps_assemble` and `aac_ld_mps_inband`, including
777384 truncated-prefix rollback checks. The197-file corpus regenerates
byte-identically (194 authored artifacts plus three retained oracle artifacts).
A separately launched broader86-suite LD/ELD run is not included in this
focused pass claim until it finishes.
