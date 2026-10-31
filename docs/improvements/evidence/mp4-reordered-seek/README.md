# FVID-0450: compact exact MP4 reorder bounds

Accepted:450/1000. One representation/seek improvement;fixture variants,measurement prototypes and the test-observer repair add no IDs.

## Result

Exact prefix maxima and suffix minima at32sample block boundaries replace the full per-sample suffix array. Local bounds are recovered exactly;safe sync selection keeps the original strict prefix-coverage rule. Local prefixes are computed once per block,and blocks without prefix growth are skipped. Retained capacity is charged to the existing reorder half-budget before decoding. No additional uncharged cache or public API is introduced. Strict ordered tracks still retain zero auxiliary payload.

## Final comparative measurements

Criteria in plan.json were fixed before production edits. Seven interleaved paired rounds after excluded warmup used60,000memory-resident authored16x16 VP9 keyframes,PTS permutation0,2,1,and1000queries. Every run asserts exact sync landings,decoded RGB and intervals. All own compilation/qualification jobs had finished before final timing.

| Metric | Baseline median | Final median |
| --- | ---: | ---: |
| Navigation |43.117708ms|0.417208ms|
| Actual software seek+RGB |70.402833ms|26.016834ms|
| Open |0.763916ms|0.712041ms|

Navigation103.35x and actual seek+RGB2.706x meet unchanged criteria;both win7/7pairs,and opening does not regress. comparison.json retains all samples,MADs,warmups,binary/workload hashes and criteria. Prototype timing/source/build/focused records are preserved separately and excluded from final acceptance. This is not a disk,GPU or whole-player speed claim.

For150000samples,the measured retained AND charged reorder index is75008bytes versus1,200,000bytes on the byte-identical baseline video implementation. Historical measurement source continuity is verified in memory-source-continuity.json. Ordered payload remains0bytes. memory.log/memory-after.json capture the final allocation ledger;the probe repeats an existing unit and adds no distinct count. Full150000frame reordered decode now passes at the unchanged2MiB decoder/reorder budget. Specific64KiB index-budget refusal and intrinsic VP9 picture-budget refusal remain. This is not total process RSS.

## Correctness and qualification

Nine new authored short videos cover3/33/96/99sample boundaries,ordered,equal,negative,descending and late-high PTS across blocks. Independent exact YUV planes,all original landing indices and decoded suffixes,native software RGB/intervals,rewind and EOF are checked.13files regenerate byte-identically;ordinary tests require no generator,encoder or network. Existing dense corpus and its independent oracles are retained.

All six final gates are terminal0:127integration suites591passes;rootMP458,WebM67,media547;1259distinct after4duplicate events. Core/player locked offline builds pass. Existing two ignores are unchanged. Focused6suites21passes and the allocation probe are excluded from that distinct total. Frozen source/corpus hashes are recorded. Temporary binaries,workload,build cache and logs were removed only after proof/hash verification;generators,fixtures and evidence remain.

## Initial full-check failure and observer repair

The first127suite attempt stopped in the unchanged MACE worker regression at144captured packets versus160expected;all five other gates passed. Initial results/logs are archived as initial-*. The test cleared its shared packet vector AFTER enqueueing rewind,which preserves playing state. A deterministic bounded probe on the original72085cf33e video/audio runtime waited for valid rewound output and then performed that same observer clear:1packet erased,159remaining,160total. Its expected assertion failure demonstrates a test observer deleting valid output,not decoder loss. The probe source,log and original source hashes are preserved;the runnable temporary probe was removed. Audio runtime,worker,helper and original MACE test matched the published baseline before reproduction.

The ordinary regression now clears its observer BEFORE enqueueing rewind. The sole edit is that setup-order swap;no expected PCM,PTS,count,seek,timeout or error assertion changed. This repair counts0. Final optimized video source was restored byte-identically,all21focused checks passed,and the entire six-gate qualification was repeated successfully. The initial failed run,prototype and expected failing observer demonstration are never counted as successful qualification.
