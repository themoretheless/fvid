# CABAC 4:4:4 transform-size neighbour contexts

Author separately: `python3 scripts/generate_avc_444_cabac_transform_switch.py`.
Test offline: `cargo test --offline --no-default-features --test avc_444_cabac_transform_switch`.

162 owned progressive High 4:4:4 Predictive level 4.1 two-macroblock CABAC
videos use one shared slice with I4/I8, I8/I4, I16/I8, I8/I16, Cb-only I4/I8
and Cb-only I8/I4 transitions. Depths 8/8, 10/12, 14/8, independent chroma
QP offsets 0/-6/+6 and deblocking IDC 0/1/2 are covered. Full-plane scalar
oracles also model filtered intra8 DC reference samples from differently sized
neighbours before loop filtering. Native and software MP4 bytes, crop/depth,
PTS/duration/index, reset and rewind are checked.

The first valid I4->I8 sample failed the initial connected decoder with
`truncated or oversized short bit field`: it had selected a coded-block context
from a neighbour's 4x4 coefficient flag. H.264 9.3.3.1.1.9 requires that
neighbour transform block to be unavailable for an 8x8 category. The corrected
Y/Cb/Cr context derivation preserves inferred one for PCM/slice-unavailable
neighbours and uses actual flags from available 8x8 transform blocks.

All 162 cases passed after that context correction on 2026-10-21. These are
acceptance tests with complete owned bitstreams, not passing refusal tests.
No FFmpeg, network, private samples or copied private codec parameters are used.
Compressed field/MBAFF and inter pictures are outside this matrix.

Final 2026-10-21 verification passed all 215 AVC unit tests and 14 selected
AVC integration targets, including ordinary monochrome/422 CABAC, inter
residuals, MBAFF loop filtering, component fields, separate-plane 8x8 scaling,
prior 444 CAVLC/PCM/filter matrices, and these CABAC cases. Reauthoring every
artifact in this directory independently reproduced identical bytes.
