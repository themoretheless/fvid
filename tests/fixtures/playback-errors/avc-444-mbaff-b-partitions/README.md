# Owned shared 4:4:4 MBAFF B partitions and component weights

Generate separately with `python3 scripts/generate_avc_444_mbaff_b_partitions.py`.
No external encoder/decoder, FFmpeg, network access, private pixels or copied
private parameter sets are used. Ordinary tests consume the saved files.

3984 synthetic I/I/B/P movies cover all 35 B partition table variants: 18
macroblock two-partition codes, 13 uniform subpartition codes and four mixed
subpartition patterns. All run with CAVLC and CABAC init 0/1/2; frame, field,
columns and checker MBAFF pairs; 8/8, 10/12 and 14/8 component depths; signed
fractional MVDs; selected per-group field references; and ASO. Plain prediction
covers every variant; 16 mixed L0/L1/bi/direct patterns additionally cover
explicit component weights, implicit frame weights and implicit selected-field
POC weights. The second intra reference uses the opposite pair topology.

The independent scalar oracle constructs each list's motion/ref grid from
already-decoded neighbours, freezes external neighbours for spatial direct,
samples every full-resolution component with six-tap qpel interpolation, then
applies uni/bi component weights and precision-aware offsets/clipping. Per-cell
reference parity controls both sampling and implicit selected-field POCs.
Witnesses reject replacing selected field POCs with a picture minimum or
ignoring opposite-field indices, and require explicit-weight effects, clipping
and actual bidirectional output.

Tests verify actual PPS flags, wire first_mb/ASO, bottom POC deltas, I/I/B/P
slice types and CABAC init. Every coded native sample and cropped normalized
software MP4 byte is compared; presentation ordering/timestamps/durations,
reset and rewind are repeated. The last P echoes the second reference.
These samples disable filtering and carry no B residuals. They do not prove
all interactions with residuals/scaling/deblocking or arbitrary temporal DPBs.
