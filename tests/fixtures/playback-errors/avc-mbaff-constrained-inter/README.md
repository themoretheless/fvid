# MBAFF constrained intra after inter

Generate separately with `python3 scripts/generate_avc_mbaff_constrained_inter.py`.
These 16 owned monochrome I/P movies cover frame and checker pairs, CAVLC and
CABAC init 0/1/2, and constrained/ordinary I16 prediction controls.

Each P pair starts with fractional-motion AC4 inter and ends in zero-residual
I16 DC. With constrained prediction the lower frame MB has no eligible top
intra samples and must predict midpoint 128. With ordinary prediction it uses
the reconstructed top edge. Field pairs have no same-field top neighbour.
The scalar witness requires constrained and ordinary pictures to differ.

Before the fix, CAVLC frame constrained output at sample 512 was 121 instead
of 128: inter publication had marked samples eligible for intra prediction.
The enabled acceptance test compares complete coded luma, software MP4 luma,
presentation timestamps, reset and rewind, without an external decoder.
Deblocking is disabled; files contain no private samples or parameter sets.
