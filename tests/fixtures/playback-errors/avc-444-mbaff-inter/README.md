# Owned shared 4:4:4 MBAFF inter

Generate separately with `python3 scripts/generate_avc_444_mbaff_inter.py`.
Ordinary Rust tests consume committed data without FFmpeg or network access.

1008 synthetic 32x64 I/P/P movies cover CAVLC and CABAC init 0/1/2,
8/8, 10/12 and 14/8 component depths, frame/field/columns/checker pairs,
selected reference parity, full-resolution quarter-pel six-tap interpolation,
explicit component weights, AC 4x4/8x8 frame/field scans, bottom skip and
I16 after inter with constrained prediction controls. Independent scalar
prediction and inverse transforms produce all three full-resolution golden
planes. Witnesses distinguish nearest sampling, wrong parity, wrong scan and
unconstrained prediction. Cropped MP4 output, timing, decoder reset and reader
rewind are checked separately from coded plane comparisons.

This matrix disables deblocking and has one slice per pair. It does not prove
all H.264 tools, profiles, partition modes or enabled deblocking support.
