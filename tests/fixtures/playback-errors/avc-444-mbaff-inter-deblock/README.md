# Owned shared 4:4:4 MBAFF inter deblocking

Generate separately with `python3 scripts/generate_avc_444_mbaff_inter_deblock.py`.
No FFmpeg, external encoder, network or private media is required.

192 synthetic I/P/P movies cover CAVLC and CABAC init 0/1/2, 8/8, 10/12 and
14/8 depths, four mixed pair topologies, AC4/AC8 residuals in every component,
and deblocking IDC 0/2. One slice covers each pair; both disabled inter-slice
edges and mixed field/frame physical edges affect the expected samples.
An independent scalar physical-owner traversal applies bS=2 for the nonzero
inter blocks, with 4x4 or 8x8 edge selection and component depth scaling.
Witnesses require actual filter changes and distinct IDC 0/2 outputs.

The first PCM picture is unfiltered. The third picture predicts the filtered
second picture without residual or filtering, checking DPB publication too.
Tests compare complete coded planes, cropped software MP4 bytes, timestamps,
durations, reset and rewind. This matrix does not qualify C-only nonzero flags,
unequal per-component QPs or all motion-derived bS combinations.
