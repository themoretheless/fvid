# Owned AVC data partitioning

Regenerate separately with `python3 scripts/generate_avc_data_partitions.py`.
The generator writes 135 artifacts (167,286 bytes before this README), uses no
external tools or network, and imports only owned bit writers/container muxers.
Ordinary tests read stored artifacts without running the generator.

Fourteen 32x16 three-picture MP4 streams use Extended profile (88), 8-bit 4:2:0
CAVLC and disabled deblocking. An ordinary PCM IDR initializes the reference
picture. Two following pictures use partition A (NAL2) plus B (NAL3, intra/PCM)
and/or C (NAL4, inter residuals) as required. Every PCM sample is nonzero, as
required by the Extended profile. All parameter sets and pixel samples are
synthetic; no private source material is copied.

The cases include I/PCM, P/inter+PCM and PCM+inter, two nonzero inter macroblocks,
zero-CBP inter with absent C, and nonzero intra4 residuals in I and mixed P
slices. Each layout has a single-slice and reverse-ordered two-slice variant.
A separate ASO packet interleaves A0,A1,C1,B0, proving slice-ID association rather
than adjacency. Coefficient tokens next to PCM use their actual nC context.

Each partitioned packet has an independently serialized combined NAL1 control.
Planar golden pixels come from scalar coordinate samples, a literal inverse-
transform basis `[4,2,-2,-4]`, and literal intra DC block predictions. Acceptance
compares every plane with both controls, through consecutive decoding/reset and
forced-software MP4 rewind, sync-seek and EOF. Thus PCM-only or header-only
acceptance is insufficient to pass this fixture.

Thirteen negative packets cover orphan, duplicate, out-of-order, wrong slice ID,
wrong redundant rank, wrong reference class, missing/truncated B/C, duplicate A
and unneeded C. Their tests require the intended rejection and successful reset
recovery. Refusals are separate from playback acceptance.

The original VCL dispatch (NAL1/5 only) was temporarily reinstated for the owned
positive regression: after the valid IDR it specifically failed with
`unsupported in-band AVC NAL`. The enabled fix reconstructs the intended pixels.
These cases do not certify all B/SP/SI, field/MBAFF, FMO, loss-recovery or
redundant-partition combinations. Changing PPS redundant-counter presence
within a partitioned access unit is now covered by the mixed-counter matrix
below (the previous refusal test has been replaced by acceptance).


Four additional `mixed-counter` videos use a primary PPS without the counter and
a redundant PPS with the counter. They reuse slice IDs across ranks 0/1/2/127,
include reversed split slices, and use redundant PCM values exactly +1 above the
primary coordinate source. Primary P residuals accumulate across two pictures;
scalar oracles and independently serialized primary-only controls must both
match every sample after reset, MP4 rewind and sync seek. Twenty-four redundant
B payloads additionally undergo manual PCM type/alignment/sample/trailing-bit
validation, so an ignored malformed redundant representation cannot qualify.
Three order regressions cover reintroduced primary A/B and descending ranks,
with short MP4 reproducers also checked through software playback and rewind.

The old `mixed-counter.packet` now passes bounded primary selection. Its original
specific refusal (`PPS changes redundant counter presence`) was reproduced before
the fix. PPS activation follows H.264 7.4.1.2.1; coded picture ordering follows
7.4.1.2.3. Changing PPS at a primary/redundant boundary is allowed; late partitions
must not borrow metadata from an earlier coded picture.
