# Owned shared 4:4:4 MBAFF P partitions

Generate separately with `python3 scripts/generate_avc_444_mbaff_p_partitions.py`.
No external codecs, FFmpeg, network or private media/parameter sets are used.

1200 synthetic I/P/P movies cover CAVLC and CABAC init 0/1/2, 8/8, 10/12 and
14/8 depths, frame/field/columns/checker pair layouts, selected reference parity,
both signs of MVDs and reversed wire slice order (ASO). P macroblocks use
16x8, 8x16, 8x8, 8x4, 4x8, 4x4, mixed subpartitions and CAVLC ref0-8x8 syntax.
Each pair is a separate slice. Every macroblock's scalar motion grid resolves
its own decoded-neighbour median or directional predictor before adding MVDs.
Each full-resolution component then samples its own plane with the same luma
six-tap quarter-pel process. A witness rejects substituting one vector for all
partitions. The independent reference pattern is deliberately nonuniform.

Acceptance compares every coded component sample and cropped MP4 byte, verifies
presentation timestamps and durations, repeats reset/rewind and checks the first
wire slice address and CABAC-init selector against manifest metadata. The echo
checks DPB retention of the reconstructed partition picture.

These samples have no residuals, weights or enabled deblocking. Their acceptance
does not prove all interactions of partitions with those tools, nor all B syntax.
