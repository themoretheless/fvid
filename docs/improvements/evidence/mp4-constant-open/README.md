# Constant MP4 open latency audit

Accepted FVID-0542 / batch553:190 passing Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All five reused short synthetic files are byte-identical on regeneration;5314 source/input hashes remain stable. Base36cf0d3628 already includes FVID-0541; its public constant-description Mp4Reader::open still performs a separate per-virtual-sample header equality scan. Reuse the authored341ms PCM/VP9 catalog fixtures; this is not another description-ID membership change or another format capability.

Predeclared performance gate: seven balanced release before/after process pairs, two warmup rounds and nine1024-open measured rounds, using Cursor<Arc<[u8]>>. Both32-description constant-open targets must win all seven paired medians and improve aggregate median at least2x; the one-description control must not regress over10%. Reader metadata/index fingerprints must agree. Scope: in-memory parser/index/constant-header opening, excluding file acquisition, decoder setup, planning and playback throughput. Ordinary tests must prove exact packet bytes/clocks/budget4, genuine changing-header refusals and scalar-guard equivalence. Core/player and related regression gates required. Reject and revert if inconclusive.

Pilot calibration: the one-description control is about1.5us/open, only0.39ms per256-open round. Before candidate changes or paired measurements, raise the final comparison to1024 opens/round for a more stable control window. Keep the same seven pairs, two warmups, nine measured rounds,2x target/all-pair wins and10% control bound. The pilot is not comparative acceptance evidence.

The candidate preserves the representative Track header check against the first selected description, then delegates the rest to the existing compact-aware header-change predicate. Empty tracks and catalogs of size0/1 preserve their previous public guard behavior. Description numbers may change without requiring reconfiguration when headers are equivalent; actually changed or opaque headers remain rejected. No public signature, catalog, index, byte offset, packet payload, clock, decoder state or budget is changed.

The shared unit compares the exact original scalar guard at168 combinations: catalogs0/1/2/32, three representative headers, compact/expanded/alias/changed/gapped/overlapping/zero/unreadable and empty shapes. This is one equivalence property, not168 improvements. The integration stand reads every packet through both native and owned constant APIs, checks exact PCM words/clocks/description IDs and the original four-record budget, and retains the typed decoder-reconfiguration refusal for a genuinely changed source. Related native playback/seek/worker gates remain required.

Five existing committed synthetic files from mp4-admission-catalog and their separate generator are reused. No new media failure or format capability is claimed; this is a separately measured constant-reader admission opportunity after the ID-membership planning optimization. Ordinary tests do not generate fixtures, invoke FFmpeg, an encoder or network, or assert timing.

## Paired release opening measurements

Microseconds per constant-reader open; medians of seven process medians. All captured reader/index fingerprints agree.

| Catalog/selected ID | Original | Candidate | Ratio | Wins |
| --- | ---: | ---: | ---: | ---: |
| catalog1-selected1.mp4 | 1.517 | 1.536 | 0.988x | 1/7 |
| catalog32-selected1.mp4 | 87.290 | 3.452 | 25.284x | 7/7 |
| catalog32-selected32.mp4 | 87.191 | 3.439 | 25.354x | 7/7 |

The single-description control median is1.25% slower, within the predeclared10% bound; it is not reported as a speedup. Both32-description targets win7/7 pairs and exceed the2x target. No file acquisition, decoder setup, planning or playback throughput is measured.
