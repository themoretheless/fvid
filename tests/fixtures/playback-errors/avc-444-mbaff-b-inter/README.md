# Owned shared 4:4:4 MBAFF B inter

Generate separately with `python3 scripts/generate_avc_444_mbaff_b_inter.py`.
No FFmpeg, private media, copied codec parameter sets or network access.

720 synthetic 32x64 I/P/B/P movies cover 8/8, 10/12 and 14/8 depths,
CAVLC and CABAC init 0/1/2, all four pair topologies, spatial/temporal direct,
bottom direct skip, explicit bi-prediction twins, unweighted, explicit
component and implicit temporal weights. Future P pairs switch frame/field
mode. Its nonzero fractional motion is converted to current field/frame units
before temporal scaling (POC 0 -> 6, B at 2). All components use full-resolution
six-tap interpolation, signed arithmetic, component precision and clipping.
A scalar temporal-versus-spatial witness ensures retained motion affects pixels.

The Rust test compares all coded Y/Cb/Cr samples in decode order, repeats after
reset, and verifies cropped normalized MP4 bytes in presentation order,
40 ms timestamps and durations, and rewind. Deblocking is disabled; this is
acceptance for the described matrix, not all H.264 B syntax or profiles.
