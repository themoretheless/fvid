# Unequal HRTF grids: exact partition analysis and remaining phase issue

The seven existing native QMF maps are exactly compositions of the23-band
QMF map and their23-to-native expansion maps. This holds for every bin in
all32/64/128-bank geometries:21 map equivalences, with zero differing bins.
All49 spatial/HRTF resolution pairs therefore have a common exact partition
with at most23 regions. Across three bank geometries there are147 exact joint
partitions. No numerical averaging/interpolation or padding is needed to
represent the pair of band indices. The adjacent generated JSON records every
partition, all phase-gate counterexamples and the source-facts digest.

For64/128 bins, the joint partition region counts are:

| Spatial / HRTF bands | 4 | 5 | 7 | 9 | 12 | 15 | 23 |
|---|---:|---:|---:|---:|---:|---:|---:|
|4|4|7|8|10|12|15|23|
|5|7|5|9|9|13|15|23|
|7|8|9|7|11|12|15|23|
|9|10|9|11|9|15|15|23|
|12|12|13|12|15|12|15|23|
|15|15|15|15|15|15|15|23|
|23|23|23|23|23|23|23|23|

This improves the implementation direction: evaluate the joint spatial/HRTF
partition rather than invent lossy conversion of HRTF levels to the coarser
spatial grid. It does not yet justify a renderer change. In particular,
index-dependent operations do not commute with this lift.

Both the pinned original ISO `Calculate3Dinv` implementation and the owned
`aac_ld_mps_3d::project` use a band-index threshold12 for HRTF phase. Naively
repeating the parameters on23 bands changes that branch in18 of21 native-map
geometries. For spatial grid4 at QMF bin12, the native index is2 while the
fine index is12. With authored center level16 and phase index4, the center
response selected by those two branches differs by approximately0.39018.
This is an analytic branch witness, not a codec stream, waveform comparison,
claim that either interpretation is normative, or proof of a new decoder bug.

A complete implementation still needs authoritative phase-band semantics,
separate spatial/HRTF indices in projection, independently checked M1/M2 and
PCM, and qualified smoothing/cutoff/history behavior across SSC changes.
An equal-grid23 encoded control can isolate level-only fixed-configuration
cases, but cannot alone resolve the phase threshold or reconfiguration history.
No narrower level-only behavior is substituted for the full requested support.
The unequal-grid refusal remains in production and no playback/conformance
acceptance is claimed by this analysis.

The original archive is the already pinned official SAOC reference source:
`b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
Its `sac_bitdec.c` uses the same resolution table for the two signalled counts,
but `Calculate3Dinv` directly indexes the directional arrays by its spatial
band; no additional conversion rule was recovered there.

Reproduce explicitly, offline, without FFmpeg or an external codec:

```sh
python3 scripts/analyze_ld_mps_hrtf_partitions.py
```

This is research evidence for the remaining implementation, not a replacement
for the synthetic playback acceptance regressions required with that fix.
