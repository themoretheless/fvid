# Owned shared 4:4:4 MBAFF temporal-direct subpartitions

Generate separately with
`python3 scripts/generate_avc_444_mbaff_b_temporal_partitions.py`.
The files and golden pixels are authored without FFmpeg, external codecs,
network access or private media/parameter sets.

1152 synthetic I/P/B/P movies cover direct-only and two mixed subpartition
patterns, both signs of future fractional motion, CAVLC and CABAC init 0/1/2,
four pair topologies, 8/8, 10/12 and 14/8 component depths, plain/explicit/
implicit/selected-field-POC weights, and reversed slice order. The future P
uses the opposite pair topology, ensuring frame/field co-located conversion.

The scalar author converts retained vertical MVs to current field/frame units,
then scales by actual POC distances before deriving L0 and L1 temporal vectors.
Those direct cells and explicit L0/L1 cells share one independent neighbour
prediction grid. All components use full-resolution six-tap sampling. The
weight combiner respects per-cell list availability, field parity, component
denominators and depth-scaled offsets. Zeroing retained direct motion must
change every case; additional witnesses distinguish field versus picture POCs,
opposite-reference POCs and clipping.

Tests assert real temporal-direct headers, future P type, PPS weighting flags,
POC deltas, CABAC init and ASO. They compare all coded native samples, cropped
normalized software-MP4 bytes, display ordering and exact 40 ms timing, and
repeat reset/rewind. A final P echoes the retained future picture. No residuals
or enabled deblocking are included; broader interactions remain unqualified.
