# Owned partitioned MBAFF SI, ordinary intra and PCM

Generate separately with `python3 scripts/generate_avc_partitioned_mbaff_si.py`.
The generator writes 5,383 artifacts (7,703,571 bytes before this README), runs
with PATH empty and external commands/network denied, and uses no private media
or copied parameter sets. Ordinary tests read stored artifacts without running
the generator or invoking FFmpeg/network.

There are 672 four-frame 32x32 Extended-profile MBAFF videos. An ordinary owned
PCM IDR seeds the stream; partitioned SI pictures use QS0/26/51. The matrix
covers all four frame/field pair layouts, seven switching/ordinary/PCM patterns,
constrained prediction on/off, signed/zero residuals, luma-only or luma+chroma
DC/AC, one/two slices and reversed/interleaved A/B arrival. PCM values are nonzero
and address/component-specific. Ordinary intra neighbours have different sample
availability rules from SI under constrained prediction.

Category2 controls and pair flags stay in A. Category3 includes SI residuals and
PCM alignment/samples in B; no C is allowed or generated. Thus SI residuals and
PCM can share B while their intervening macroblock controls remain in A. B is
absent when a slice has neither coded SI residual nor PCM. Signed CAVLC tokens
use actual same-slice physical contexts, including PCM count16. Controls are
serialized by the original owned combined-stream SI writer; all 3,360 combined
slices were additionally checked byte-identical to that original path.

Golden pixels use independently implemented per-sample availability labels,
spatial DC and integer switching matrices. Every component sample must match
both goldens and combined-stream decoding through reset. Forced-software MP4
checks four complete pictures, exact 40ms PTS/duration, EOF, rewind and sync seek.
The metadata test validates 3,360 actual SI headers, constrained flag, QP/QS,
pair-unit IDs, B presence/offset and exact byte bounds. A separate manual A trace
checks each field flag, SI/ordinary/PCM type, prediction flag, CBP and QP delta
through the trailing bits. Constrained/unconstrained goldens are independently
required to differ where switching neighbours matter.

The current native decoder passed this finite matrix without a production
change. This does not establish all geometries, FMO, weighting/motion/filter
combinations, missing-primary recovery, complete H.264 or all codec coverage.
