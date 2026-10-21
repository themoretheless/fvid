# Owned FMO slice data partitioning

Generate separately using `python3 scripts/generate_avc_partitioned_fmo.py`.
The generator writes 741 artifacts (1,350,374 bytes before this README) and runs
with PATH empty and external commands/network denied. Ordinary tests only read
stored artifacts. SPS/PPS, samples and residuals are original synthetic data;
no private source images, video, audio or parameter sets are copied.

The matrix has 120 three-picture 32x32 Extended-profile videos. It covers FMO
map types0–6, both directions for changing maps3/4/5 (change-cycle2), coded,
PCM/inter mixed and PCM/skip mixed macroblocks. Each case has either one slice
per group or a separate slice for every macroblock. The ASO variant reverses
slice order and interleaves all A before B before C NALs, requiring association
by slice ID independently of physical adjacency or sorted macroblock ranges.

Each group has opposite signed C residuals: a literal inverse-transform basis
`[4,2,-2,-4]` for group0 and its negative for group1. Consequently swapping C
payload ownership or ignoring a direction/group-map change alters the pixels.
PCM uses nonzero coordinate samples. Residual coefficient tokens use actual
same-slice left/top contexts, including PCM nC16 and mixed nC9, and reset across
slice boundaries. Inter prediction is zero-motion from the previously decoded
reference, so repeated signed residuals have a simple independent scalar oracle.

Native tests compare every component sample with scalar golden data and an
independently serialized combined-stream control over two reset passes.
Software MP4 tests check all three pictures, exact 40ms PTS/duration, EOF,
rewind and sync seek. A separate test checks 720 headers: actual P/FMO map,
literal map-unit values, cycle, unique slice IDs, associated B/C entropy offsets,
required/absent C and exact packet byte limits. The golden sample check separately
confirms opposite group signs over two reference updates.

This finite matrix passed the existing native decoder without a production
change. It does not establish arbitrary dimensions, more than two groups,
all residual/motion/weight combinations, B/SP/SI with FMO, field/MBAFF, deblocking
or loss recovery. Codec-wide completion remains unproven.
