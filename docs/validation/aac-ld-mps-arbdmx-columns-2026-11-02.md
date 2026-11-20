# Per-input arbitrary-downmix compensation for trees5/6/7

An authored nonzero ELD MP4 previously failed specifically with the arbitrary-
downmix/HRTF matrix-composition refusal. The corresponding programme without
ADG decoded successfully in the same prior executable. Both outputs and the
executable hash are retained in the receipt.

The OTT-only matrix path now applies the decoded gain for each input column
before matrix-time smoothing and interpolation. Same-format assembler
reconfiguration is exposed as an API with interpolation history retained. A six-input tree uses six
independent DownmixGain regions; mono212 uses its one region. The previously
qualified mono515 paths use the same column rule with one input. TTT gains
continue through their existing pre-inversion builder. M2, output PCM and
post-shaping histories are not scaled by this compensation. Downmix-residual
extensions and unqualified HRTF/tree combinations keep their existing refusal.

## Numeric qualification

On/off/on headers are applied to the public same-format assembler API before
each frame, with the Decimal oracle retaining its previous endpoints. This
checks that gain-mode changes preserve interpolation state.

The base M1 matrices for7571/7572 and212 have independently known sparse input
routing: six identity rows plus two decorrelation feeds for7571/7572, or two
feeds of the mono input for212. The authored oracle builds gain endpoints and
per-slot interpolation using80-digit Decimal exponentiation and recurrence,
without invoking the matrix builder or production decoder. Tests expand this
sparse oracle into every real/imaginary matrix coefficient and compare all
endpoints and slots across three frames. The tree6 feeds use inputs4/5 rather
than0/1, making wrong-column compensation observable. Every encoded gain index
and row is parsed and compared to the authoring metadata. M2 dry/wet matrices
must remain byte-identical to the matching ADG-disabled control.

A separate explicit C probe calls the unchanged original CalculateArbDmxMtx for
one and six inputs with distinct per-channel gains. All49 conversion values
agree with independent Decimal powers within2.06e-7. This qualifies the dB-to-
gain component only. The original757/212 routines omit this ADG composition;
no foreign whole-matrix or whole-decoder agreement is claimed for these paths.
The new composition is the per-input diagonal compensation applied to the
already qualified base M1 routing, with temporal behavior independently tested.
Original source remains external and is never linked into production or called
by ordinary tests.

## Media and navigation

378 own three-AU programmes cover trees5/6/7, all seven parameter grids,
unity, independently varying per-input/time/frequency gains, in-band on/off/on
switches and all six
qualified PCM/shared-LD-SBR domains.126 ADG-disabled controls isolate signalling
and input geometry. Unity signalling must produce exactly the control PCM;
varying gains must change it. Both containers must produce identical nonzero
PCM with exact source duration. Both decorrelation modes check timestamps,
checkpoint restore and truncated-packet retry. Reader tests cover full playback,
nine seek boundaries and rewind. All fixtures contain own generated video,
core data and parameter values, with no private media or ISO conformance data.

The authored generator is separate from tests and requires no process or network.
Ordinary tests load saved media and Decimal oracle and do not require FFmpeg,
reference compilation or network access. The focused gate and rebuilt device
probe results are recorded in adjacent JSON receipts. This does not establish
full-current-tree qualification or complete codec parity.

The initial adapted clock test assumed arbitrary-tree mask0; these ordinary
output trees correctly expose their known masks (0xff,0x63f,3). The expectation
was corrected. The former5/6/7 constructor-refusal test is updated to admission;
full numeric and playback acceptance is supplied by the new corpus, rather
than treating a refusal test as playback support. No tolerances were loosened.

Qualification completed:26 unique targeted tests pass without ignored tests.
The new six-test378-case target and seven-test TTT target are terminal successes.
The unchanged mono and older switch targets contribute5 and8 passing tests from
the preceding mixed run. Its obsolete constructor-refusal expectation failed;
that TTT result is explicitly superseded, not reported as a green job.
10,054,800 Decimal matrix coefficient comparisons have maximum error
8.881784197001252e-16 (limit2e-10). All1,011 authored files regenerate identically
with processes and sockets forbidden. The rebuilt default-device probe completed
12 runs and48 drains across all six domains and all three trees, including gain
mode switches. This is delivery evidence, not subjective listening or physical
speaker-wiring certification.
