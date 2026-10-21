# Redundant-picture and disjoint-region recovery

Owned 32x16 Extended-profile streams generated separately by
`scripts/generate_avc_redundant_recovery.py`. No private media/parameters,
FFmpeg, external commands or network are used. Ordinary tests read checked-in
artifacts and never generate them.

84 accepted cases cover single/split/ASO slices, ordinary and A/B/C data
partitioning, rank 1/2/127, lowest complete rank, incomplete/late-start/truncated
lower ranks, alternate PPS selection, intra PCM and P zero-CBP recovery, IDR
startup and non-reference recovery. Scalar nonzero planar samples and separately
written rank-zero controls qualify the recovered picture and two successors.
MP4 checks exact 40 ms clocks, frame count, EOF, rewind and sync seek.
Preparation checks source rank, original RBSP entropy offsets, byte/context
bounds and the old strict-primary helper's specific refusal.

Five negative packets cover incomplete representations, truncation,
foreign picture/reference identity and descending ranks. These are refusal
regressions. The former disjoint-ranks refusal is now an acceptance test. Reset is required
after an error and verified to restore decoding.

Disjoint slices sharing one PPS and reference marking can now be composed
across ranks. Lower ranks retain ownership where higher-rank slices overlap;
nonoverlapping higher-rank slices fill holes. If an overlapping higher slice
cannot be used without cutting its entropy/prediction context, an available
whole representation retains the previous whole-picture fallback. This is not
yet sample-level priority across partially overlapping slices. Different PPS,
recovery inside a damaged slice/header and arbitrary profiles/geometries/entropy tools
remain outside this finite matrix.

The independent rank-zero controls now preserve each selected slice's original
boundaries and prediction type. Scalar goldens distinguish +2 low-rank PCM
areas from +1 higher-rank PCM areas and unchanged P zero-CBP areas. Reversed
spatial rank order and rank 127 are covered. All 837 generated artifacts
(785,523 bytes) reproduce with external commands/network disabled.

Independently valid slices now survive a damaged neighbour in the same rank.
Truncated split/ASO goldens and controls retain the intact lower-rank half and
replace only the missing half. Ordinary single-slice truncation still requires
another complete representation.
